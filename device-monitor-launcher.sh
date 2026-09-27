#!/bin/sh
# 单服务启动器：API 常驻 + 显示链降级。
#   显示优先级：Slint GPU 面板（panel）→ 旧 DRM 横屏（--screen）→ kmscon UTF-8 TUI → 裸 ASCII TUI
# 由 systemd device-monitor.service 调用，保证 API 始终可用。
#
# 可用环境变量：
#   SCREEN_ROTATE=90|270        横屏旋转方向（默认 90；未显式设置时读 rotation.txt 兜底）
#   DEVICE_MONITOR_FORCE_SCREEN=1 跳过 GPU 面板，直接用旧 --screen 渲染
#   DEVICE_MONITOR_FORCE_TUI=1  跳过两条 DRM 路径，直接用 kmscon
#   DEVICE_MONITOR_FORCE_ASCII=1 直接裸 ASCII TUI
#   TUI_FONT_SIZE                kmscon 字号
#   SLINT_SCALE_FACTOR           面板缩放（默认 2 → 逻辑 1170x540）

DEPLOY_DIR="$(cd "$(dirname "$0")" && pwd)"
BIN="$DEPLOY_DIR/target/release/device-monitor-server"
PANEL_BIN="$DEPLOY_DIR/target/release/device-monitor-panel"
WRAPPER="$DEPLOY_DIR/.device-monitor-tui-wrapper.sh"
SCREEN_LOG="$DEPLOY_DIR/screen.log"
PANEL_LOG="$DEPLOY_DIR/panel.log"
API_LOG="$DEPLOY_DIR/api.log"
LOG_TAG="device-monitor-launcher"
KMSCON="/usr/libexec/kmscon/kmscon"
[ -x "$KMSCON" ] || KMSCON="/usr/bin/kmscon"

log() {
  echo "[$LOG_TAG] $*"
  logger -t "$LOG_TAG" "$*" 2>/dev/null || true
}

cleanup_kmscon() {
  pkill -f 'kmscon.*device-monitor' 2>/dev/null || true
  pkill -f 'kmscon.*device-monitor-tui-wrapper' 2>/dev/null || true
  sleep 1
}

health_check() {
  curl -sf --connect-timeout 2 --max-time 3 \
    http://127.0.0.1:3000/api/system/overview >/dev/null 2>&1
}

fix_devpts() {
  if [ -e /dev/pts/ptmx ]; then
    mount -o remount,mode=620,gid=5,ptmxmode=0666 /dev/pts 2>/dev/null || true
  fi
}

# 按屏幕分辨率估算 kmscon 字号。优先保证横向约 80 列，避免表格和长行换行。
calc_font_size() {
  if [ -n "${TUI_FONT_SIZE:-}" ]; then
    echo "$TUI_FONT_SIZE"
    return
  fi
  w=""
  h=""
  if [ -r /sys/class/graphics/fb0/virtual_size ]; then
    V=$(cat /sys/class/graphics/fb0/virtual_size 2>/dev/null)
    w="${V%,*}"
    h="${V##*,}"
  fi
  if { [ -z "$w" ] || [ -z "$h" ]; } && [ -r /sys/class/drm/card0-DSI-1/modes ]; then
    mode=$(head -1 /sys/class/drm/card0-DSI-1/modes 2>/dev/null)
    case "$mode" in
      *x*)
        w="${mode%x*}"
        h="${mode#*x}"
        ;;
    esac
  fi
  [ -z "$w" ] && w=1080
  [ -z "$h" ] && h=2340

  target_cols="${TUI_TARGET_COLS:-82}"
  target_rows="${TUI_TARGET_ROWS:-72}"
  # CJK monospace cell width is roughly 0.56 * font-size, line height roughly 1.25 * font-size.
  by_width=$((w * 100 / (target_cols * 56)))
  by_height=$((h * 100 / (target_rows * 125)))
  font="$by_width"
  [ "$by_height" -lt "$font" ] && font="$by_height"
  [ "$font" -lt 20 ] && font=20
  [ "$font" -gt 30 ] && font=30
  echo "$font"
}

write_wrapper() {
  cat > "$WRAPPER" <<EOF
#!/bin/sh
export LANG=zh_CN.UTF-8
export LC_ALL=zh_CN.UTF-8
export TUI_UTF8=1
export RUST_LOG=\${RUST_LOG:-info}
export PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
exec "$BIN" --tui --tty -
EOF
  chmod +x "$WRAPPER"
}

fallback_ascii() {
  log "Falling back to ASCII TUI"
  bind_fbcon
  cleanup_kmscon
  printf '\033[2J\033[H' > /dev/tty1 2>/dev/null || true
  export TUI_UTF8=0
  exec "$BIN" --tui
}

# ── API 常驻：面板数据源。--screen 模式自带 API，走旧链前要先杀掉这个实例 ──
API_PID=""
start_api() {
  [ -x "$BIN" ] || { log "server binary not found: $BIN"; return 1; }
  if health_check; then
    log "API already healthy (external instance?)"
    return 0
  fi
  env LANG=zh_CN.UTF-8 LC_ALL=zh_CN.UTF-8 TUI_UTF8=1 RUST_LOG="${RUST_LOG:-info}" \
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

stop_api() {
  if [ -n "$API_PID" ] && kill -0 "$API_PID" 2>/dev/null; then
    kill "$API_PID" 2>/dev/null || true
    wait "$API_PID" 2>/dev/null || true
    log "API instance stopped (旧链自带 API)"
  fi
  API_PID=""
}

# 解绑内核 framebuffer 控制台（fbcon）：否则我们服务的 stdout（tty1）、
# 内核 printk、以及面板 unblank 都会让 fbcon 把控制台画到屏幕上，抢走画面。
unbind_fbcon() {
  if [ -w /sys/class/vtconsole/vtcon1/bind ]; then
    echo 0 > /sys/class/vtconsole/vtcon1/bind 2>/dev/null && log "fbcon 已解绑（避免控制台抢屏）"
  fi
}

# 回退到字符 TUI 前必须把 fbcon 绑回来，否则控制台是哑设备、什么也显示不出来
bind_fbcon() {
  if [ -w /sys/class/vtconsole/vtcon1/bind ]; then
    echo 1 > /sys/class/vtconsole/vtcon1/bind 2>/dev/null && log "fbcon 已重新绑定"
  fi
}

# 触摸校准矩阵与屏幕朝向联动：Slint linuxkms 只旋转渲染、不对输入坐标做任何变换
# （后端源码注释：implemented entirely inside the actual renderer），libinput 的
# 校准矩阵是唯一修正点。两个横向朝向的矩阵互为 180° 对偶；规则内容与当前朝向
# 不符时重写并触发 udev，面板随后打开输入设备即读到新矩阵。
sync_touch_matrix() {
  case "${1:-90}" in
    270) _m="0 -1 1 1 0 0" ;; # x=1-ty, y=tx
    *) _m="0 1 0 -1 0 1" ;;   # x=ty, y=1-tx（实测校准基准）
  esac
  _rule=/etc/udev/rules.d/70-stmfts-rotation.rules
  _want='# stmfts 触摸校准矩阵 —— 由 device-monitor-launcher.sh 随屏幕朝向自动维护（矩阵行勿手改）
# rot90 = 0 1 0 -1 0 1（基准，2026-09-27 实测）；rot270 = 0 -1 1 1 0 0（180° 对偶）
ACTION=="add|change", KERNEL=="event*", ATTRS{name}=="stmfts", ENV{LIBINPUT_CALIBRATION_MATRIX}="'"$_m"'"
'
  if [ "$(cat "$_rule" 2>/dev/null)" != "$(printf '%s' "$_want")" ]; then
    printf '%s' "$_want" > "$_rule"
    udevadm control --reload-rules >/dev/null 2>&1 || true
    udevadm trigger --action=change --subsystem-match=input >/dev/null 2>&1 || true
    udevadm settle --timeout=5 >/dev/null 2>&1 || true
    log "触摸校准矩阵已随朝向同步（rotate=$1 → $_m）"
  fi

  # 属性存在性兜底：模块重载/新节点时 udev 对 add 的处理可能滞后数秒甚至更久，
  # 面板一旦先打开设备就会读不到矩阵。这里在启动面板前确认当前实例已带上
  # 属性，缺了就重试注入（trigger 用 sysfs 路径，/dev 路径在此处无效）。
  _ev_sys=""
  for _p in /sys/class/input/event*; do
    [ "$(cat "$_p/device/name" 2>/dev/null)" = "stmfts" ] && _ev_sys="$_p" && break
  done
  if [ -n "$_ev_sys" ]; then
    _tries=0
    while [ "$_tries" -lt 10 ] && ! udevadm info "$_ev_sys" 2>/dev/null | grep -q "LIBINPUT_CALIBRATION_MATRIX=$_m"; do
      _tries=$((_tries + 1))
      udevadm trigger --action=change "$_ev_sys" >/dev/null 2>&1 || true
      udevadm settle --timeout=3 >/dev/null 2>&1 || true
      sleep 1
    done
    if [ "$_tries" -ge 10 ]; then
      log "警告: 触摸矩阵属性注入失败（已重试 10 次），触摸映射可能不对"
    elif [ "$_tries" -gt 0 ]; then
      log "触摸矩阵属性晚到，已注入（重试 $_tries 次）"
    fi
  fi
  return 0
}

# ── 首选：Slint GPU 面板（femtovg 直驱 GLES，数据走本机 API）──
try_panel() {
  if [ "${DEVICE_MONITOR_FORCE_TUI:-0}" = "1" ]; then
    log "Skipping GPU panel (DEVICE_MONITOR_FORCE_TUI=1)"
    return 1
  fi
  if [ "${DEVICE_MONITOR_FORCE_SCREEN:-0}" = "1" ]; then
    log "Skipping GPU panel (DEVICE_MONITOR_FORCE_SCREEN=1)"
    return 1
  fi
  [ -x "$PANEL_BIN" ] || { log "panel binary not found: $PANEL_BIN"; return 1; }

  cleanup_kmscon
  unbind_fbcon
  # 朝向：SCREEN_ROTATE 显式设置 > rotation.txt（旧渲染器留下的用户朝向）> 90。
  rotate="${SCREEN_ROTATE:-}"
  if [ -z "$rotate" ] && [ -r "$DEPLOY_DIR/rotation.txt" ]; then
    rotate=$(tr -dc '0-9' < "$DEPLOY_DIR/rotation.txt" 2>/dev/null)
  fi
  case "$rotate" in
    90 | 270) ;;
    *) rotate=90 ;;
  esac
  # 触摸坐标修正必须与朝向一致，否则触摸整体反 180°（见函数注释）
  sync_touch_matrix "$rotate"

  log "Attempting Slint GPU panel (SLINT_KMS_ROTATION=$rotate)"
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
  wait "$panel_pid"
  return $?
}

# ── 回退 1：旧 DRM/KMS 直绘横屏仪表（自绘像素排版，不依赖 kmscon/终端字体）──
try_screen() {
  if [ "${DEVICE_MONITOR_FORCE_TUI:-0}" = "1" ]; then
    log "Skipping DRM screen (DEVICE_MONITOR_FORCE_TUI=1)"
    return 1
  fi
  [ -x "$BIN" ] || { log "binary not found: $BIN"; return 1; }

  cleanup_kmscon
  unbind_fbcon
  # 朝向只在 SCREEN_ROTATE 显式设置时才传给程序（当作一次性覆盖）。
  # 不能无条件给个默认值：程序里「命令行 > rotation.txt > Rot90」，
  # 启动器硬塞 --rotate 90 会让双击音量加存下的朝向每次开机都被覆盖掉，
  # 表现为「自定义朝向重启就丢」。
  rotate_args=""
  rotate="持久化值"
  if [ -n "${SCREEN_ROTATE:-}" ]; then
    rotate="$SCREEN_ROTATE"
    rotate_args="--rotate $SCREEN_ROTATE"
  fi
  log "Attempting DRM landscape screen (--screen ${rotate_args:-无 --rotate，用 rotation.txt})"
  : > "$SCREEN_LOG"
  env LANG=zh_CN.UTF-8 LC_ALL=zh_CN.UTF-8 TUI_UTF8=1 RUST_LOG="${RUST_LOG:-info}" \
    PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin \
    "$BIN" --screen $rotate_args >>"$SCREEN_LOG" 2>&1 &
  screen_pid=$!

  i=0
  while [ "$i" -lt 20 ]; do
    sleep 1
    i=$((i + 1))
    if ! kill -0 "$screen_pid" 2>/dev/null; then
      log "DRM screen exited early (see $SCREEN_LOG)"
      wait "$screen_pid" 2>/dev/null || true
      return 1
    fi
    if health_check; then
      log "DRM landscape screen healthy (pid=$screen_pid, rotate=$rotate)"
      wait "$screen_pid"
      return $?
    fi
  done

  log "DRM screen health check timed out after 20s"
  kill "$screen_pid" 2>/dev/null || true
  wait "$screen_pid" 2>/dev/null || true
  return 1
}

# ── 回退 2：kmscon UTF-8 竖屏 TUI ──
try_kmscon() {
  bind_fbcon
  [ -x "$KMSCON" ] || { log "kmscon not found, skip UTF-8"; return 1; }
  [ -x "$BIN" ] || { log "binary not found: $BIN"; return 1; }

  fix_devpts
  cleanup_kmscon
  write_wrapper

  font_size=$(calc_font_size)
  log "Attempting UTF-8 kmscon launch via $KMSCON (font-size=${font_size})"
  env LANG=zh_CN.UTF-8 LC_ALL=zh_CN.UTF-8 TUI_UTF8=1 RUST_LOG="${RUST_LOG:-info}" \
    "$KMSCON" --no-libseat --vt=1 \
    --font-name="Noto Sans CJK SC" \
    --font-size="$font_size" \
    --dpms-timeout=0 \
    -l -- "$WRAPPER" &
  kms_pid=$!

  i=0
  while [ "$i" -lt 15 ]; do
    sleep 1
    i=$((i + 1))
    if health_check; then
      log "UTF-8 kmscon healthy (kmscon pid=$kms_pid)"
      wait "$kms_pid"
      return $?
    fi
    if ! kill -0 "$kms_pid" 2>/dev/null; then
      log "kmscon exited before server became healthy"
      cleanup_kmscon
      return 1
    fi
  done

  log "kmscon health check timed out after 15s"
  kill "$kms_pid" 2>/dev/null || true
  wait "$kms_pid" 2>/dev/null || true
  cleanup_kmscon
  return 1
}

# 强制 ASCII：DEVICE_MONITOR_FORCE_ASCII=1
if [ "${DEVICE_MONITOR_FORCE_ASCII:-0}" = "1" ]; then
  fallback_ascii
fi

# 1) API 常驻 + GPU 面板
if start_api; then
  if try_panel; then
    exit 0
  fi
  # 面板没起来或中途退出：旧 --screen 自带 API，先让出 3000 端口
  stop_api
else
  log "API unavailable; skipping GPU panel"
fi

# 2) 旧 DRM 横屏仪表
if try_screen; then
  exit 0
fi

# 3) kmscon UTF-8 竖屏 TUI
if try_kmscon; then
  exit 0
fi

# 4) 裸 ASCII TUI
fallback_ascii
