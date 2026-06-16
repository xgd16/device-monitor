#!/bin/sh
# Fix sysfs permissions for device-monitor hardware controls
# Run as root at boot: sudo /home/user/device-monitor/setup-permissions.sh

# Flashlight LEDs
chmod 666 /sys/class/leds/white:flash/brightness
chmod 666 /sys/class/leds/yellow:flash/brightness

# Status LED
chmod 666 /sys/class/leds/white:status/brightness

# Backlight (screen brightness + power)
chmod 666 /sys/class/backlight/ae94000.dsi.0/brightness
chmod 666 /sys/class/backlight/ae94000.dsi.0/bl_power

# Charging current limit
chmod 666 /sys/class/power_supply/pmi8998-charger/current_max 2>/dev/null || true

# GPU devfreq
chmod 666 /sys/class/devfreq/5000000.gpu/max_freq 2>/dev/null || true
chmod 666 /sys/class/devfreq/5000000.gpu/min_freq 2>/dev/null || true

echo "Hardware sysfs permissions fixed."
