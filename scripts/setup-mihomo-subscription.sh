#!/bin/sh
# 安装 mihomo 订阅自动更新（需 root）
# 用法: sudo ./setup-mihomo-subscription.sh

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
FETCH_SCRIPT="/home/user/code/fetch_sub.py"
CRON_TAG="device-monitor-mihomo-sub"

if [ "$(id -u)" -ne 0 ]; then
  echo "请使用 root 运行: sudo $0"
  exit 1
fi

mkdir -p /etc/mihomo /home/user/code

cp "$SCRIPT_DIR/scripts/fetch_sub.py" "$FETCH_SCRIPT"
cp "$SCRIPT_DIR/scripts/mihomo-local-overrides.yaml" "/home/user/code/mihomo-local-overrides.yaml"
chmod +x "$FETCH_SCRIPT"
cp "$SCRIPT_DIR/scripts/mihomo-local-overrides.yaml" /etc/mihomo/local-overrides.yaml

# 每天 04:00 自动更新订阅
CRON_LINE="0 4 * * * /usr/bin/python3 $FETCH_SCRIPT >> /var/log/mihomo-sub-update.log 2>&1 # $CRON_TAG"

if crontab -l 2>/dev/null | grep -q "$CRON_TAG"; then
  crontab -l 2>/dev/null | grep -v "$CRON_TAG" | crontab -
fi
( crontab -l 2>/dev/null; echo "$CRON_LINE" ) | crontab -

echo "已安装:"
echo "  脚本: $FETCH_SCRIPT"
echo "  覆盖: /etc/mihomo/local-overrides.yaml"
echo "  定时: 每天 04:00 自动更新"
echo ""
echo "立即更新: python3 $FETCH_SCRIPT"
