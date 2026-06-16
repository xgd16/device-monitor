# kmscon TUI UTF-8 适配记录

本文记录本项目在 Xiaomi Mi Mix 3 / postmarketOS 上，为物理屏 TUI 启用 UTF-8 中文显示时遇到的问题、最终方案和维护命令。

## 背景

默认 TUI 直接写 `/dev/tty1`。裸 Linux VT 没有 CJK 字库，中文会乱码或显示方块，所以早期 TUI 在裸 TTY 下使用 ASCII/英文告警。

为了在手机本机屏幕上显示中文，需要使用 `kmscon` 作为 KMS/DRM 终端，并加载 Noto CJK 字体：

```bash
apk add --no-cache kmscon font-noto-cjk
```

## 最终方案

保持单个 `device-monitor.service`，不拆成多个 systemd 服务。服务通过启动器脚本启动：

```text
device-monitor.service
  -> device-monitor-launcher.sh
     -> kmscon
        -> device-monitor-server --tui --tty -
```

关键文件：

- `device-monitor-launcher.sh`
- `setup-tui-utf8.sh`
- `/etc/systemd/system/device-monitor.service.d/launcher.conf`
- `src/tui/mod.rs`

当前 systemd drop-in：

```ini
[Service]
Environment=RUST_LOG=info
Environment=TUI_FONT_SIZE=12
ExecStart=
ExecStart=/home/user/code/device-monitor/device-monitor-launcher.sh
StandardInput=tty
StandardOutput=tty
TTYPath=/dev/tty1
TTYReset=yes
TTYVHangup=yes
```

当前 kmscon 启动参数：

```bash
/usr/libexec/kmscon/kmscon \
  --no-libseat \
  --vt=1 \
  --font-name="Noto Sans CJK SC" \
  --font-size=12 \
  --dpms-timeout=0 \
  -l -- /home/user/code/device-monitor/.device-monitor-tui-wrapper.sh
```

wrapper 会设置：

```bash
LANG=zh_CN.UTF-8
LC_ALL=zh_CN.UTF-8
TUI_UTF8=1
PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
```

## 关键问题与修复

### 1. 直接用 kmscon 覆盖 ExecStart 会导致服务不可用

早期尝试：

```ini
ExecStart=/usr/bin/kmscon --font=... -- device-monitor-server --tui --tty -
```

问题：

- `kmscon` 参数语义与预期不同，容易启动成 login 或没有拉起子进程。
- 即使 `kmscon` 进程存在，`device-monitor-server` 可能没起来，API 返回 `HTTP 000`。

最终修复：

- 使用 `device-monitor-launcher.sh`。
- 启动器先尝试 kmscon。
- 15 秒内用 `http://127.0.0.1:3000/api/system/overview` 做健康检查。
- 健康检查失败时自动杀掉 kmscon，回退到普通 ASCII TUI：

```bash
device-monitor-server --tui
```

### 2. devpts 权限导致 PTY 子会话异常

设备上曾出现：

```text
/dev/pts/ptmx: ptmxmode=000
```

可能导致 kmscon 创建 PTY 失败。

启动器中做了保守修复：

```bash
mount -o remount,mode=620,gid=5,ptmxmode=0666 /dev/pts
```

失败也不会中断启动。

### 3. 字号和屏幕比例

Mi Mix 3 屏幕为：

```text
1080x2340
```

测试过程：

- `font-size=44`：字太大，横向列数太少，表格大量折行。
- `font-size=23`：仍可能显示内容不完整。
- `font-size=10`：行列数多，但字偏小。
- 最终使用 `font-size=12`。

字号通过 systemd 环境变量固定：

```ini
Environment=TUI_FONT_SIZE=12
```

如果要临时调整：

```bash
sed -i 's/^Environment=TUI_FONT_SIZE=.*/Environment=TUI_FONT_SIZE=12/' \
  /etc/systemd/system/device-monitor.service.d/launcher.conf
systemctl daemon-reload
systemctl restart device-monitor
```

### 4. kmscon 自动熄屏

现象：

- 手机屏幕过一段时间自动黑屏。
- Web 后台点击亮屏无效。
- `/sys/class/backlight/ae94000.dsi.0/bl_power` 变成 `4`。

原因：

- `kmscon` 默认 DPMS 空闲超时会关闭显示输出。

修复：

```bash
--dpms-timeout=0
```

当前启动器已经包含该参数。

### 5. 后端命令 PATH 缺失

现象：

- TUI 显示 WiFi 未连接，但实际已连接。
- Web 后台无法控制 WiFi 省电模式。
- API 报错：

```text
iw failed: No such file or directory
```

原因：

- kmscon 启动的子进程环境里没有完整 `PATH`。
- `device-monitor-server` 执行 `iw` 找不到命令。

修复：

- wrapper 显式设置完整 `PATH`。
- Rust 代码中调用 `iw`、`ip`、`nmcli` 时优先使用绝对路径。

### 6. 光标显示在屏幕底部

现象：

- TUI 底部出现类似等待输入的方块。

原因：

- 那是终端光标，不是 shell 提示符。

修复：

`src/tui/mod.rs` 每帧：

- 隐藏光标：`\x1b[?25l`
- 关闭自动换行：`\x1b[?7l`
- 清屏并回到左上角：`\x1b[2J\x1b[H`
- 帧尾把光标移动到右下角并继续隐藏。

### 7. Unicode 图标兼容性

一开始使用了 `⚠`、`◆`、`▦`、`◫` 等符号，部分在 kmscon/Noto CJK 组合下显示不稳定。

最终改为 CJK 字体覆盖更稳定的中文标签：

```text
【设备】
【CPU】
【GPU】
【硬件】
【内存】
【核心】
【电池】
【温度】
【网络】
【无线】
【告警】
【磁盘】
【进程】
```

进度条使用 UTF-8 块字符：

```text
██████▌░░░░░
```

ASCII 回退模式仍使用：

```text
[-----.....]
```

## TUI 渲染策略

UTF-8 模式：

- 中文文案。
- 温度显示为 `°C`。
- 进度条使用块字符。
- 使用中文标签代替 emoji 图标。

ASCII 回退模式：

- 英文/ASCII 文案。
- 温度显示为 `C`。
- 不输出中文和 Unicode 图标。

判断逻辑：

```text
TUI_UTF8=1/true/yes/on -> UTF-8 模式
TUI_UTF8=0/false/no/off -> ASCII 模式
否则根据 LANG/LC_ALL 是否包含 utf-8 判断
```

## 常用命令

查看服务状态：

```bash
systemctl status device-monitor --no-pager
```

查看当前 kmscon 参数：

```bash
pgrep -af kmscon
```

确认 API 正常：

```bash
curl -s -o /dev/null -w 'HTTP %{http_code}\n' \
  http://127.0.0.1:3000/api/system/overview
```

重启服务：

```bash
systemctl restart device-monitor
```

重新配置 UTF-8 TUI：

```bash
cd /home/user/code/device-monitor
sh setup-tui-utf8.sh
systemctl restart device-monitor
```

强制 ASCII 回退：

```bash
DEVICE_MONITOR_FORCE_ASCII=1 systemctl restart device-monitor
```

移除启动器，恢复直接启动：

```bash
rm /etc/systemd/system/device-monitor.service.d/launcher.conf
systemctl daemon-reload
systemctl restart device-monitor
```

## 当前已知状态

设备当前运行链路：

```text
device-monitor.service
  -> /home/user/code/device-monitor/device-monitor-launcher.sh
  -> kmscon --font-size=12 --dpms-timeout=0
  -> device-monitor-server --tui --tty -
```

服务验证项：

```text
systemctl is-active device-monitor -> active
/api/system/overview -> HTTP 200
kmscon 使用 /dev/dri/card0
```

## 维护建议

- 不要再直接把 `device-monitor.service` 的 `ExecStart` 改成裸 `kmscon ... device-monitor-server`。
- 修改 TUI 字号优先改 `TUI_FONT_SIZE`，不要改 Rust 布局代码。
- 若屏幕黑屏，先检查 `kmscon` 是否带 `--dpms-timeout=0`。
- 若 WiFi、GPU 等命令型功能异常，先检查服务进程环境里的 `PATH`。
- 若出现方块字符，优先替换为中文标签或 ASCII，不要依赖 emoji。
