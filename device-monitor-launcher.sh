#!/bin/sh
# 单服务启动器：优先启动 DRM 横屏仪表（--screen），失败依次回退 kmscon UTF-8 TUI、裸 ASCII TUI。
# 由 systemd device-monitor.service 调用，保证 API 始终可用。
#
# 可用环境变量：
#   SCREEN_ROTATE=90|270        横屏旋转方向（默认 90）
#   DEVICE_MONITOR_FORCE_TUI=1  跳过 DRM 横屏，直接用 kmscon
#   DEVICE_MONITOR_FORCE_ASCII=1 直接裸 ASCII TUI
#   TUI_FONT_SIZE                kmscon 字号

DEPLOY_DIR="$(cd "$(dirname "$0")" && pwd)"
BIN="$DEPLOY_DIR/target/release/device-monitor-server"
WRAPPER="$DEPLOY_DIR/.device-monitor-tui-wrapper.sh"
SCREEN_LOG="$DEPLOY_DIR/screen.log"
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

# 首选：DRM/KMS 直绘横屏仪表（自绘像素排版，不依赖 kmscon/终端字体）
try_screen() {
  if [ "${DEVICE_MONITOR_FORCE_TUI:-0}" = "1" ]; then
    log "Skipping DRM screen (DEVICE_MONITOR_FORCE_TUI=1)"
    return 1
  fi
  [ -x "$BIN" ] || { log "binary not found: $BIN"; return 1; }

  cleanup_kmscon
  # 解绑内核 framebuffer 控制台（fbcon）：否则我们服务的 stdout（tty1）、
  # 内核 printk、以及面板 unblank 都会让 fbcon 把控制台画到面板上，抢走画面。
  if [ -w /sys/class/vtconsole/vtcon1/bind ]; then
    echo 0 > /sys/class/vtconsole/vtcon1/bind 2>/dev/null && log "fbcon 已解绑（避免控制台抢屏）"
  fi
  rotate="${SCREEN_ROTATE:-90}"
  log "Attempting DRM landscape screen (--screen --rotate $rotate)"
  : > "$SCREEN_LOG"
  env LANG=zh_CN.UTF-8 LC_ALL=zh_CN.UTF-8 TUI_UTF8=1 RUST_LOG="${RUST_LOG:-info}" \
    PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin \
    "$BIN" --screen --rotate "$rotate" >>"$SCREEN_LOG" 2>&1 &
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

# 回退到字符 TUI 前必须把 fbcon 绑回来，否则控制台是哑设备、什么也显示不出来
bind_fbcon() {
  if [ -w /sys/class/vtconsole/vtcon1/bind ]; then
    echo 1 > /sys/class/vtconsole/vtcon1/bind 2>/dev/null && log "fbcon 已重新绑定"
  fi
}

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

# 1) DRM 横屏仪表
if try_screen; then
  exit 0
fi

# 2) kmscon UTF-8 竖屏 TUI
if try_kmscon; then
  exit 0
fi

# 3) 裸 ASCII TUI
fallback_ascii
