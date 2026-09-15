# 物理屏横屏仪表（DRM/KMS 直绘）

本文记录把设备**本机屏幕**上的仪表从「kmscon + ANSI 竖屏字符网格」改为
「DRM/KMS 直绘 + 像素级排版横屏卡片」的方案、原理、坑与运维命令。

对应代码：`src/screen/`，入口 `device-monitor-server --screen [--rotate 90|270]`。

## 为什么不能直接硬件旋转

MSM/DPU 的 DRM plane 只暴露 0/180 度：

```bash
modetest -M msm -p | grep -A2 rotation
# 42 rotation:  flags: bitmask
#    values: rotate-0=0x1 rotate-180=0x4 reflect-x=0x10 reflect-y=0x20
```

没有 rotate-90/270，也没有可用的面板横屏模式（唯一模式 1080x2340）。
`rotate-180 + reflect` 是二面体群里的 4 个操作，**凑不出转置**，因此真横屏只能软件旋转。

## 软件旋转怎么做到不卡

朴素做法是「每帧把 2340x1080 的 10MB 画面转置到 1080x2340」，手机上缓存不友好、开销大。

本实现改为**在字形光栅化阶段一次性预旋转**并缓存：

- 逻辑画布 2340x1080，物理扫描缓冲 1080x2340，两者互为转置
- 映射（`Rotation::map_point` / `map_rect`）：

  | 旋转 | 逻辑 (x,y) → 物理 (px,py) | 说明 |
  |------|--------------------------|------|
  | `Rot90` | `px = y`, `py = lw-1-x` | 手机**顺时针转 90°**（左边朝上）观看 |
  | `Rot270` | `px = lh-1-y`, `py = x` | 若画面上下颠倒就换这个 |

- 字形位图在缓存时按目标旋转方向重排（`font.rs::rotate_cov`）：
  - `Rot90`：`tile[ty][tx] = cov[gy=tx][gx=lw-1-ty]`
  - `Rot270`：`tile[ty][tx] = cov[gy=lh-1-tx][gx=ty]`

于是绘制时按**物理行连续写内存**，与正屏绘制同价；矩形填充同理（逻辑矩形 → 物理块）。
只有圆角弧、折线这类小范围为逐像素写入。

## 提交机制：CMD 面板必须逐帧 commit（本项目最大的坑）

面板是 **CMD 模式** DSI（设备树名 `dsi_samsung_fhd_ea8076_cmd_display`）。
CMD 模式不像 video 模式那样持续扫描显存：**写完 dumb buffer 必须显式提交，帧才会出现在屏上**。

最初实现是「单缓冲 + 零提交」，于是面板永远收不到新帧，一直停留在**上一次被推上去的画面**。
本次实测那块残帧是几个月前某个下游 5.10-Android 内核 panic 的日志（黑底左上角一小块白字），
看起来非常像「黑屏 / 只能显示终端」，极具误导性。

最终实现（`display.rs`）：
- **双缓冲 + `drmModePageFlip`**（kmscon 当年就是靠它工作的）作为主路径
- `drmModeDirtyFB`（damage 上报）作为回退
- 因为要 flip，绘制源必须是**完整画布**（每帧整帧拷贝，1Hz 下约 3–5ms，可忽略）

**验证提交是否真的生效**（不看屏也能判断）：
```bash
modetest -M msm -p | sed -n '4p'   # 间隔几秒采样两次
# 生效：CRTC 上的 fb 句柄在两个 buffer 之间交替（如 90 → 88 → 90 …）
# 未生效：句柄始终不变
```

## 渲染与刷新

- 单缓冲直写 + **脏矩形差分刷新**：数据变化（5s）整屏重绘，其余每秒只重绘顶栏时钟
- 每帧只 `memcpy` 脏矩形到扫描缓冲，撕裂窗口极小
- 排版规范取自 `tui-design` skill（Widget Dashboard 范式、语义色板 `theme.rs`、
  层次：卡片标题 muted+Bold、主数值 emphasis+超大字号、颜色只做语义提示且数值始终同时呈现）

## 启动链路（三级回退）

`device-monitor-launcher.sh`：

```
1) --screen（DRM 直绘横屏）      ← 首选
2) kmscon + --tui（竖屏字符 TUI） ← 回退
3) --tui（裸 VT ASCII）          ← 最后
```

判据是 20 秒内 `http://127.0.0.1:3000/api/system/overview` 是否 200；
`--screen` 初始化失败会 `exit(3)`，让启动器继续往下回退（避免「API 活着但屏幕空白」）。

环境变量：
- `SCREEN_ROTATE=90|270` 旋转方向
- `DEVICE_MONITOR_FORCE_TUI=1` 跳过 DRM 横屏
- `DEVICE_MONITOR_FORCE_ASCII=1` 直接用裸 ASCII TUI

启动器会把 `--screen` 进程的 stdout/stderr 重定向到 `screen.log`（否则日志经 tty1
会触发 fbcon 重绘，把画面盖掉）。

## 离屏导出（无 DRM，可在服务运行时执行）

```bash
device-monitor-server --screen-dump /tmp/scr.ppm
# 产出：/tmp/scr.ppm        逻辑方向（横屏所见，2340x1080）
#       /tmp/scr.ppm.raw.ppm 物理方向（面板实际扫描内容，1080x2340）
ffmpeg -y -i /tmp/scr.ppm /tmp/scr.png
```

两份互为 90° 旋转，既是预览手段，也是**验证旋转映射是否正确**的手段
（改完版式/旋转后先看导出图，不用反复盯物理屏）。

## 坑

### 1. 次要隐患：msm DSI 把背光 `bl_power` 当 DPMS 用
（注意：本次「黑屏」的真因是上面那节「CMD 面板必须显式提交」，不是这条。
这条是排查过程中发现的独立隐患，同样会导致黑屏，已一并加固。）
电源键单击 = 写 `/sys/class/backlight/ae94000.dsi.0/bl_power` 为 `4`（灭）/`0`（亮）。
在 KMS 客户端自己持有输出的情况下，**写回 0 不会自动恢复画面**，且服务启动瞬间
电源键监听可能收到一次点击，导致一开机就是黑屏。

对策：
- 启动时显式 `set_screen_power(true)` 并把 `power_key` 状态同步为「亮」
- 渲染循环里每秒做 CRTC 看门狗：`get_crtc()` 的 framebuffer 不是我们的就
  `reassert()`（重跑 `set_crtc`）+ 整屏重绘；同时读 `power_key::is_screen_on()`
  尊重用户主动灭屏，不对着干

### 2. 字体缺字形会画成豆腐块
`ab_glyph` 对字体里没有的字符返回 `GlyphId(0)`（`.notdef`），照画就是空方块。
代理节点名常带 ✅/⭐ 这类 BMP 符号，Noto Sans CJK 没有。
对策：`glyph_id == 0` 时**不绘制**、只保留步进（`font.rs`）。

### 3. 自引用结构：`DumbBuffer` 与它的映射
`map_dumb_buffer(&mut db)` 返回 `DumbMapping<'a>` 借用 `db`，同时把 `db` 存进结构体
会变成两个可变借用（E0499）。做法：`Box::leak(Box::new(db))` 得到 `&'static mut`，
把它**直接移交给** `DumbMapping`，结构体里不再保留 `_db` 字段（泄漏即保活）。

### 4. `tracing` 宏里的 `display` 函数名冲突
`tracing::info!("{}", display.field)` 中局部变量若命名为 `display`，会与宏内部
`field::display` 冲突（报「no field on type fn(_) -> DisplayValue」）。局部变量改名即可。

### 5. 两阶段借用：`c.text(..., &truncate(c, ...))`
`&mut self` 方法（`truncate(&mut Canvas)`）不能内联在另一个 `&mut self` 调用
（`c.text(...)`）的参数里。先把结果取到局部变量再传。

### 6. drm 0.15 crate 的具体 API 名
| 需求 | 正确写法 |
|------|----------|
| 拿 master | `card.acquire_master_lock()`（不是 `set_master`） |
| 连接器类型 | `connector::Interface::DSI` |
| 编码器可用 CRTC | `res.filter_crtcs(enc.possible_crtcs())`（不是 `enc.crtcs()`） |
| Handle → u32 | `u32::from(handle)` |

### 7. 依赖与编译
需要 `apk add libdrm-dev pkgconf`（`drm` crate 经 `drm-ffi` 链接 libdrm）；
SDM845 上 release 全量编译约 10 分钟。

## 运维命令

```bash
# 编译部署
cd ~/code/device-monitor && cargo build --release && systemctl restart device-monitor

# 看日志（渲染/DRM/自愈）
tail -f ~/code/device-monitor/screen.log

# 确认是我们持有输出
ls -l /proc/$(pgrep -f 'device-monitor-server --screen')/fd | grep card0
modetest -M msm -p | sed -n '2,4p'      # CRTC 上的 fb 应为我们

# 导出预览
./target/release/device-monitor-server --screen-dump /tmp/scr.ppm

# 换旋转方向 / 回退旧界面
echo 'Environment=SCREEN_ROTATE=270' >> /etc/systemd/system/device-monitor.service.d/launcher.conf
echo 'Environment=DEVICE_MONITOR_FORCE_TUI=1' >> /etc/systemd/system/device-monitor.service.d/launcher.conf
systemctl daemon-reload && systemctl restart device-monitor
```

## 收益

- 字号 12px → 19–62px（大数值 62px），6.39" 屏上清晰可读
- 从字符网格升级为像素排版：圆角卡片、胶囊标签、真折线趋势图、阈值配色
- 同一屏承载 10 张卡片（CPU/GPU/温度/内存/磁盘/进程/电池/网络/服务告警）+ 顶栏时钟 + 底栏
- 去掉 kmscon 后服务常驻内存 **1023MB → 70MB**
