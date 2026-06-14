#!/bin/sh
# Fix sysfs permissions for device-monitor hardware controls
# Run as root at boot: sudo /home/user/device-monitor/setup-permissions.sh

# Flashlight LEDs
chmod 666 /sys/class/leds/white:flash/brightness
chmod 666 /sys/class/leds/yellow:flash/brightness

# Backlight (screen brightness + power)
chmod 666 /sys/class/backlight/ae94000.dsi.0/brightness
chmod 666 /sys/class/backlight/ae94000.dsi.0/bl_power

echo "Hardware sysfs permissions fixed."
