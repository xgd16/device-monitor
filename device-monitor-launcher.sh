#!/bin/sh
# 单服务启动器：API 常驻 + Slint GPU 面板（唯一显示）。
#   2026-09-27 起移除旧回退链（DRM 横屏 --screen / kmscon UTF-8 TUI / 裸 ASCII）：
#   屏幕显示只保留 GPU 面板；面板异常退出会自动重新拉起，API 始终保持可用。
# 由 systemd device-monitor.service 调用（见 drop-in launcher.conf）。
#
# 可用环境变量：
#   SCREEN_ROTATE=90|270        横屏旋转方向（默认 90；未显式设置时读 rotation.txt 兜底）
#   SLINT_SCALE_FACTOR           面板缩放（默认 2 → 逻辑 1170x540）

DEPLOY_DIR="$(cd "$(dirname "$0")" && pwd)"
BIN="$DEPLOY_DIR/target/release/device-monitor-server"
PANEL_BIN="$DEPLOY_DIR/target/release/device-monitor-panel"
PANEL_LOG="$DEPLOY_DIR/panel.log"
API_LOG="$DEPLOY_DIR/api.log"
LOG_TAG="device-monitor-launcher"

log() {
  echo "[$LOG_TAG] $*"
  logger -t "$LOG_TAG" "$*" 2>/dev/null || true
}

health_check() {
  curl -sf --connect-timeout 2 --max-time 3 \
    http://127.0.0.1:3000/api/system/overview >/dev/null 2>&1
}

# ── API 常驻：GPU 面板与 Web 的数据源 ──
API_PID=""
start_api() {
  [ -x "$BIN" ] || { log "server binary not found: $BIN"; return 1; }
  if health_check; then
    log "API already healthy (external instance?)"
    return 0
  fi
  env LANG=zh_CN.UTF-8 LC_ALL=zh_CN.UTF-8 RUST_LOG="${RUST_LOG:-info}" \
    PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin \
    "$BIN" >>"$API_LOG" 2>&1 &
    API_PID=$!
  i=0
  while [ "$i" -lt 20 ]; do
    sleep 1
    i=$((i + 1))
    if health_check; then
      log "API healthy (pid=$API_PID)"
      return 0
    fi
    if ! kill -0 "$API_PID" 2>/dev/null; then
      log "API server exited early (see $API_LOG)"
      wait "$API_PID" 2>/dev/null || true
      API_PID=""
      return 1
    fi
  done
  log "API health check timed out after 20s"
  kill "$API_PID" 2>/dev/null || true
  wait "$API_PID" 2>/dev/null || true
  API_PID=""
  return 1
}

# 解绑内核 framebuffer 控制台（fbcon）：否则服务的 stdout（tty1）、内核 printk、
# 以及面板 unblank 都会让 fbcon 把控制台画到屏幕上，抢走面板画面。
unbind_fbcon() {
  if [ -w /sys/class/vtconsole/vtcon1/bind ] && [ "$(cat /sys/class/vtconsole/vtcon1/bind 2>/dev/null)" != "0" ]; then
    echo 0 > /sys/class/vtconsole/vtcon1/bind 2>/dev/null && log "fbcon 已解绑（避免控制台抢屏）"
  fi
}

# 触摸校准矩阵与屏幕朝向联动：Slint linuxkms 只旋转渲染、不对输入坐标做任何变换
# （后端源码注释：implemented entirely inside the actual renderer），libinput 的
# 校准矩阵是唯一修正点。横屏两个朝向互为 180° 对偶；竖屏 0 = 原生恒等、180 = 全翻转。
sync_touch_matrix() {
  case "${1:-90}" in
    270) _m="0 -1 1 1 0 0" ;; # x=1-ty, y=tx
    0) _m="1 0 0 0 1 0" ;;    # 竖屏原生：px=tx, py=ty（由 rot90 基准推导）
    180) _m="-1 0 1 0 -1 1" ;; # 竖屏倒置：px=1-tx, py=1-ty
    *) _m="0 1 0 -1 0 1" ;;   # x=ty, y=1-tx（实测校准基准）
  esac
  _rule=/etc/udev/rules.d/70-stmfts-rotation.rules
  _want='# stmfts 触摸校准矩阵 —— 由 device-monitor-launcher.sh 随屏幕朝向自动维护（矩阵行勿手改）
# rot90 = 0 1 0 -1 0 1（基准，2026-09-27 实测）；rot270 = 0 -1 1 1 0 0（180° 对偶）
# rot0 = 1 0 0 0 1 0（竖屏原生）；rot180 = -1 0 1 0 -1 1（竖屏倒置）
# 注意六元组 = a b c / d e f：x'=a*x+b*y+c，y'=d*x+e*y+f（libinput 行主序）
ACTION=="add|change", KERNEL=="event*", ATTRS{name}=="stmfts", ENV{LIBINPUT_CALIBRATION_MATRIX}="'"$_m"'"
'
  # stmfts 事件节点（下面的内容分支与存在性兜底都要用）
  _ev_sys=""
  for _p in /sys/class/input/event*; do
    [ "$(cat "$_p/device/name" 2>/dev/null)" = "stmfts" ] && _ev_sys="$_p" && break
  done
  if [ "$(cat "$_rule" 2>/dev/null)" != "$(printf '%s' "$_want")" ]; then
    printf '%s' "$_want" > "$_rule"
    udevadm control --reload-rules >/dev/null 2>&1 || true
    udevadm trigger --action=change --subsystem-match=input >/dev/null 2>&1 || true
    udevadm settle --timeout=5 >/dev/null 2>&1 || true
    log "触摸校准矩阵已随朝向同步（rotate=$1 → $_m）"
  else
    # 内容一致 = 面板退出前的预写脚本已写好并触发过：先纯轮询等属性进
    # udev 库（此时再 trigger 会让 udev 重新排队、属性反而更晚到）。
    _w=0
    while [ "$_w" -lt 8 ] && ! udevadm info "$_ev_sys" 2>/dev/null | grep -q "LIBINPUT_CALIBRATION_MATRIX=$_m"; do
      _w=$((_w + 1))
      sleep 0.3
    done
  fi

  # 属性存在性兜底：模块重载/新节点时 udev 对 add 的处理可能滞后数秒甚至更久，
  # 面板一旦先打开设备就会读不到矩阵。这里在启动面板前确认当前实例已带上
  # 属性，缺了就重试注入（trigger 用 sysfs 路径，/dev 路径在此处无效）。
  if [ -n "$_ev_sys" ]; then
    _tries=0
    while [ "$_tries" -lt 10 ] && ! udevadm info "$_ev_sys" 2>/dev/null | grep -q "LIBINPUT_CALIBRATION_MATRIX=$_m"; do
      _tries=$((_tries + 1))
      udevadm trigger --action=change "$_ev_sys" >/dev/null 2>&1 || true
      # 快轮询（0.3s×10）替代 settle(≤3s)+sleep(1s)：属性通常在 trigger 后
      # 不到 1 秒进 udev 库；面板退出前的预写脚本此刻多半已把属性备好。
      _w=0
      while [ "$_w" -lt 10 ] && ! udevadm info "$_ev_sys" 2>/dev/null | grep -q "LIBINPUT_CALIBRATION_MATRIX=$_m"; do
        _w=$((_w + 1))
        sleep 0.3
      done
    done
    if [ "$_tries" -ge 10 ]; then
      log "警告: 触摸矩阵属性注入失败（已重试 10 次），触摸映射可能不对"
    elif [ "$_tries" -gt 0 ]; then
      log "触摸矩阵属性晚到，已注入（重试 $_tries 次）"
    fi
  fi
  return 0
}

# ── Slint GPU 面板（唯一显示）：femtovg 直驱 GLES，数据走本机 API ──
try_panel() {
  [ -x "$PANEL_BIN" ] || { log "panel binary not found: $PANEL_BIN"; return 1; }

  unbind_fbcon
  # 朝向：SCREEN_ROTATE 显式设置 > rotation.txt（用户持久化朝向）> 90
  rotate="${SCREEN_ROTATE:-}"
  if [ -z "$rotate" ] && [ -r "$DEPLOY_DIR/rotation.txt" ]; then
    rotate=$(tr -dc '0-9' < "$DEPLOY_DIR/rotation.txt" 2>/dev/null)
  fi
  case "$rotate" in
    90 | 270) ;;
    *) rotate=90 ;;
  esac
  # 竖屏模式由面板设置页手动选择并持久化（panel-orientation.txt：portrait/landscape）：
  # 竖屏旋转 = 横屏朝向的对偶（270→0 / 90→180），触摸矩阵随实际旋转联动。
  _mode=""
  if [ -r "$DEPLOY_DIR/panel-orientation.txt" ]; then
    _pref=$(tr -d ' \t\n' < "$DEPLOY_DIR/panel-orientation.txt" 2>/dev/null)
    case "$_pref" in portrait | landscape) _mode=$_pref ;; esac
  fi
  if [ "$_mode" = portrait ]; then
    case "$rotate" in
      90) rotate=180 ;;
      *) rotate=0 ;;
    esac
  fi
  # 屏幕反转（panel-flip.txt = 1）：在当前朝向上再翻 180°（0↔180 / 90↔270）
  if [ -r "$DEPLOY_DIR/panel-flip.txt" ] && [ "$(tr -d ' \t\n' < "$DEPLOY_DIR/panel-flip.txt")" = "1" ]; then
    case "$rotate" in
      0) rotate=180 ;;
      180) rotate=0 ;;
      90) rotate=270 ;;
      *) rotate=90 ;;
    esac
  fi
  # 触摸坐标修正必须与朝向一致，否则触摸整体反 180°（见函数注释）
  sync_touch_matrix "$rotate"

  log "启动 Slint GPU 面板 (SLINT_KMS_ROTATION=$rotate)"
  : > "$PANEL_LOG"
  env LANG=zh_CN.UTF-8 LC_ALL=zh_CN.UTF-8 RUST_LOG="${RUST_LOG:-info}" \
    SLINT_KMS_ROTATION="$rotate" SLINT_SCALE_FACTOR="${SLINT_SCALE_FACTOR:-2}" \
    PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin \
    "$PANEL_BIN" >>"$PANEL_LOG" 2>&1 &
  panel_pid=$!

  # 面板不提供 HTTP，健康标准 = 存活跑过 12 秒（DRAM 初始化/EGL 失败都会在几秒内退出）
  i=0
  while [ "$i" -lt 12 ]; do
    sleep 1
    i=$((i + 1))
    if ! kill -0 "$panel_pid" 2>/dev/null; then
      log "GPU panel exited early (see $PANEL_LOG)"
      wait "$panel_pid" 2>/dev/null || true
      return 1
    fi
  done
  log "GPU panel running (pid=$panel_pid, rotate=$rotate)"
  # 过渡 splash 使命完成（新面板已跑过 12s 健康窗口，首帧早已接管屏幕）：
  # 击杀并清标记，灰色帧彼时已不被扫描，释放无闪烁。
  if [ -f /tmp/panel-splash.pid ]; then
    _sp=$(cat /tmp/panel-splash.pid 2>/dev/null)
    [ -n "$_sp" ] && kill "$_sp" 2>/dev/null || true
    rm -f /tmp/panel-splash.pid /tmp/panel-splash.gone
  fi
  wait "$panel_pid"
  return $?
}

# ── 主流程 ──
# 1) API 常驻（面板与 Web 的数据源；失败则退出，交给 systemd Restart=always 重试）
if ! start_api; then
  log "API 启动失败，退出（systemd 将自动重启）"
  exit 1
fi

# 2) Slint GPU 面板（唯一显示；退出则按场景重拉，API 不中断）
#    rc=0 = 设置页计划内切换：splash 已在位、矩阵已预同步，1 秒即可重拉；
#    rc≠0 = 异常退出：保留 5 秒退避，避免崩溃快速循环。
while : ; do
  try_panel
  _rc=$?
  if [ "$_rc" -eq 0 ]; then
    log "GPU panel 已退出（rc=0，计划内切换），1 秒后按新方向拉起"
    sleep 1
  else
    log "GPU panel 已退出（rc=$_rc），5 秒后重新拉起"
    sleep 5
  fi
done
