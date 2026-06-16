#!/bin/sh
# 安装 TUI UTF-8 依赖，并配置单服务启动器（kmscon 优先，失败自动回退 ASCII）。
#
# 用法: sh setup-tui-utf8.sh

set -e
cd "$(dirname "$0")"

DEPLOY_DIR="$(pwd)"
BIN="$DEPLOY_DIR/target/release/device-monitor-server"
LAUNCHER="$DEPLOY_DIR/device-monitor-launcher.sh"

echo "Installing kmscon + Noto CJK fonts..."
apk add --no-cache kmscon font-noto-cjk 2>/dev/null || {
  echo "Warning: apk install had errors; continuing if packages are present"
}

if ! command -v kmscon >/dev/null 2>&1 && [ ! -x /usr/libexec/kmscon/kmscon ]; then
  echo "kmscon not available; launcher will use ASCII fallback only"
fi

if [ ! -x "$LAUNCHER" ]; then
  echo "Error: launcher not found: $LAUNCHER"
  exit 1
fi
chmod +x "$LAUNCHER"

mkdir -p /etc/systemd/system/device-monitor.service.d
cat > /etc/systemd/system/device-monitor.service.d/launcher.conf <<EOF
[Service]
Environment=RUST_LOG=info
Environment=TUI_FONT_SIZE=12
ExecStart=
ExecStart=$LAUNCHER
StandardInput=tty
StandardOutput=tty
TTYPath=/dev/tty1
TTYReset=yes
TTYVHangup=yes
EOF

systemctl daemon-reload
echo ""
echo "Launcher configured: $LAUNCHER"
echo "  - Tries kmscon UTF-8 first (health check on :3000)"
echo "  - Falls back to ASCII TUI if kmscon fails"
echo ""
echo "Restart service:"
echo "  systemctl restart device-monitor"
echo ""
echo "Force ASCII only:"
echo "  DEVICE_MONITOR_FORCE_ASCII=1 systemctl restart device-monitor"
echo ""
echo "Revert to direct binary (remove launcher):"
echo "  rm /etc/systemd/system/device-monitor.service.d/launcher.conf"
echo "  systemctl daemon-reload && systemctl restart device-monitor"
