#!/bin/bash
# Device Monitor - 启动脚本

cd "$(dirname "$0")"

echo "🔧 Building frontend..."
cd device-monitor-web && npm run build && cd ..

echo "🦀 Building backend..."
cargo build --release 2>/dev/null || cargo build

echo "🚀 Starting server on http://0.0.0.0:3001"
./target/release/device-monitor-server 2>/dev/null || ./target/debug/device-monitor-server
