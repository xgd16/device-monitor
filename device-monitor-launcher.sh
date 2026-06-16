#!/bin/sh
# 单服务启动器：优先尝试 kmscon UTF-8 TUI，失败则自动回退 ASCII TUI。
# 由 systemd device-monitor.service 调用，保证 API 始终可用。

DEPLOY_DIR="$(cd "$(dirname "$0")" && pwd)"
BIN="$DEPLOY_DIR/target/release/device-monitor-server"
WRAPPER="$DEPLOY_DIR/.device-monitor-tui-wrapper.sh"
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
  cleanup_kmscon
  printf '\033[2J\033[H' > /dev/tty1 2>/dev/null || true
  export TUI_UTF8=0
  exec "$BIN" --tui
}

try_kmscon() {
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

if try_kmscon; then
  exit 0
fi

fallback_ascii
