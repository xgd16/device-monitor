#!/bin/sh
# 停掉未使用的 CDSP remoteproc（modem 不能停：ath10k 数据面依赖它；
# ADSP 负责音频；SLPI 被 hexagonrpcd-sdsp 使用）。
for r in /sys/class/remoteproc/remoteproc*; do
    [ "$(cat "$r/name" 2>/dev/null)" = "cdsp" ] || continue
    state=$(cat "$r/state" 2>/dev/null)
    [ "$state" = "running" ] && echo stop > "$r/state"
    echo "cdsp $(cat "$r/state" 2>/dev/null)"
done
