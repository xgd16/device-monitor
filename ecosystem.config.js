module.exports = {
  apps: [
    {
      name: 'device-monitor',
      script: './target/release/device-monitor-server',
      cwd: '/home/user/device-monitor',
      instances: 1,
      autorestart: true,
      watch: false,
      max_memory_restart: '200M',
      env: {
        NODE_ENV: 'production',
        RUST_LOG: 'info',
      },
    },
  ],
};
