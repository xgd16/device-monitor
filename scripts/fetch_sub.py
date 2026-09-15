#!/usr/bin/env python3
"""
拉取 mihomo 订阅并写入 /etc/mihomo/config.yaml，随后重启服务。

用法:
  python3 fetch_sub.py              # 使用内置订阅地址并应用
  python3 fetch_sub.py --fetch-only # 仅下载到 subscription_config.yaml
  python3 fetch_sub.py --dry-run    # 拉取并合并，不写入、不重启
"""

from __future__ import annotations

import argparse
import shutil
import subprocess
import sys
import time
from datetime import datetime, timezone
from pathlib import Path

import requests
import yaml

# 固定订阅地址（按顺序尝试，首个失败则回退）
SUBSCRIPTION_URLS = [
    "http://47.242.55.240/link/U6jzieNUGnBJbkGP?clash=2",
    "http://43.135.28.238/link/U6jzieNUGnBJbkGP?clash=2",
]
SUBSCRIPTION_URL = SUBSCRIPTION_URLS[0]

MIHOMO_DIR = Path("/etc/mihomo")
CONFIG_PATH = MIHOMO_DIR / "config.yaml"
OVERRIDES_PATH = MIHOMO_DIR / "local-overrides.yaml"
UI_INDEX_PATH = MIHOMO_DIR / "ui" / "index.html"
LAST_UPDATE_PATH = MIHOMO_DIR / ".last_subscription_update"
BACKUP_PATH = MIHOMO_DIR / "config.yaml.bak"
FETCH_ONLY_OUTPUT = Path("subscription_config.yaml")

CLASH_HTTP_PORT = 7890
CONTROLLER = "http://127.0.0.1:9090"
DELAY_TEST_URL = "http://cp.cloudflare.com/generate_204"
MIHOMO_SERVICE = "mihomo"

# 订阅不含这些项时，必须从 local-overrides 或旧配置保留
REQUIRED_LOCAL_KEYS = ("tun", "dns", "external-ui")


def patch_delay_test_urls(data: dict) -> None:
    """订阅默认用 gstatic 测速，国内 DNS 污染会导致全部超时。"""
    patched = 0
    for group in data.get("proxy-groups", []):
        if group.get("type") not in ("url-test", "fallback", "load-balance"):
            continue
        if "url" not in group:
            continue
        if group.get("url") != DELAY_TEST_URL:
            group["url"] = DELAY_TEST_URL
            patched += 1
        if group.get("type") == "url-test" and group.get("interval", 0) > 3600:
            group["interval"] = 300
    if patched:
        print(f"[修正] {patched} 个策略组测速 URL -> {DELAY_TEST_URL}")


def validate_dns_for_fake_ip(data: dict) -> list[str]:
    """fake-ip 模式缺少 fake-ip-filter 时，延迟测试会全部失败。"""
    warnings: list[str] = []
    dns = data.get("dns") or {}
    if dns.get("enable") and dns.get("enhanced-mode") == "fake-ip":
        filters = dns.get("fake-ip-filter") or []
        if not filters:
            warnings.append("dns.fake-ip-filter 为空，延迟测试可能失败")
        policy = dns.get("nameserver-policy") or {}
        if not any("wowodekuku.com" in str(k) for k in policy):
            warnings.append("dns.nameserver-policy 未包含 wowodekuku.com，节点域名解析可能异常")
    return warnings


def try_fetch(url: str, proxy: dict | None, timeout: int = 30) -> requests.Response | None:
    headers = {"User-Agent": "clash-verge/v2.5.2"}
    try:
        resp = requests.get(
            url,
            headers=headers,
            proxies=proxy,
            allow_redirects=True,
            timeout=timeout,
        )
        if resp.status_code == 200:
            return resp
        print(f"    HTTP {resp.status_code}")
        return None
    except requests.exceptions.ProxyError:
        print("    代理连接失败")
        return None
    except requests.exceptions.ConnectionError as e:
        print(f"    连接失败: {e}")
        return None
    except requests.exceptions.Timeout:
        print("    超时")
        return None
    except Exception as e:
        print(f"    {type(e).__name__}: {e}")
        return None


def fetch_subscription(url: str, clash_port: int = CLASH_HTTP_PORT) -> tuple[dict, str] | None:
    strategies = [
        ("直连", None),
        (
            f"Clash代理 (127.0.0.1:{clash_port})",
            {
                "http": f"http://127.0.0.1:{clash_port}",
                "https": f"http://127.0.0.1:{clash_port}",
            },
        ),
    ]

    resp = None
    for name, proxy in strategies:
        print(f"\n{'=' * 50}")
        print(f"[尝试] {name} -> {url}")
        print(f"{'=' * 50}")
        for attempt in range(1, 4):
            print(f"  第 {attempt} 次...")
            resp = try_fetch(url, proxy)
            if resp:
                print("  ✅ 成功!")
                break
            if attempt < 3:
                time.sleep(2)
        if resp:
            break

    if resp is None:
        return None

    body = resp.text
    if body.startswith("\ufeff"):
        body = body[1:]

    sub_info = resp.headers.get("subscription-userinfo")
    if sub_info:
        print(f"\n[订阅信息] {sub_info}")

    try:
        data = yaml.safe_load(body)
    except yaml.YAMLError as e:
        print(f"[错误] YAML 解析失败: {e}")
        return None

    if not isinstance(data, dict):
        print("[错误] 响应不是有效的 YAML 字典")
        return None

    if "proxies" not in data and "proxy-providers" not in data:
        print("[错误] 配置不包含 proxies 或 proxy-providers")
        return None

    proxy_count = len(data.get("proxies", []))
    print(f"\n[概要] 节点 {proxy_count} 个, 规则 {len(data.get('rules', []))} 条")
    return data, body


def fetch_subscription_with_fallback(urls: list[str]) -> tuple[dict, str] | None:
    for url in urls:
        print(f"\n[订阅源] {url}")
        result = fetch_subscription(url)
        if result is not None:
            return result
        print(f"[回退] {url} 不可用，尝试下一个地址...")
    print("\n[错误] 所有订阅地址均失败")
    return None


def bundled_overrides_path() -> Path | None:
    """脚本同目录下的 mihomo-local-overrides.yaml（仓库部署时可用）。"""
    path = Path(__file__).resolve().parent / "mihomo-local-overrides.yaml"
    return path if path.is_file() else None


def sync_overrides_file() -> None:
    """将 bundled overrides 同步到 /etc/mihomo，确保设备上配置与仓库一致。"""
    bundled = bundled_overrides_path()
    if bundled is None:
        return
    MIHOMO_DIR.mkdir(parents=True, exist_ok=True)
    if (
        not OVERRIDES_PATH.is_file()
        or bundled.read_bytes() != OVERRIDES_PATH.read_bytes()
    ):
        shutil.copy2(bundled, OVERRIDES_PATH)
        print(f"[同步] {bundled.name} -> {OVERRIDES_PATH}")


def load_overrides() -> dict:
    sync_overrides_file()
    for path in (OVERRIDES_PATH, bundled_overrides_path()):
        if path and path.is_file():
            with path.open(encoding="utf-8") as f:
                overrides = yaml.safe_load(f) or {}
                if isinstance(overrides, dict):
                    return overrides
    return {}


def load_old_config() -> dict:
    if CONFIG_PATH.is_file():
        old = yaml.safe_load(CONFIG_PATH.read_text(encoding="utf-8"))
        if isinstance(old, dict):
            return old
    return {}


def merge_config(sub_data: dict, overrides: dict, old: dict | None = None) -> dict:
    merged = dict(sub_data)
    for key, value in overrides.items():
        merged[key] = value

    # overrides 缺失时，从旧配置兜底保留关键本地项
    if old:
        for key in REQUIRED_LOCAL_KEYS:
            if key not in merged and key in old:
                merged[key] = old[key]
                print(f"[兜底] 从旧配置保留 {key}")

    # MetaCubeXD 面板目录存在则确保挂载
    if UI_INDEX_PATH.is_file() and not merged.get("external-ui"):
        merged["external-ui"] = "ui"
        print("[兜底] external-ui: ui")

    patch_delay_test_urls(merged)
    return merged


def write_config(data: dict) -> None:
    MIHOMO_DIR.mkdir(parents=True, exist_ok=True)
    if CONFIG_PATH.is_file():
        shutil.copy2(CONFIG_PATH, BACKUP_PATH)
        print(f"[备份] {BACKUP_PATH}")

    with CONFIG_PATH.open("w", encoding="utf-8") as f:
        yaml.dump(
            data,
            f,
            allow_unicode=True,
            default_flow_style=False,
            sort_keys=False,
        )
    print(f"[写入] {CONFIG_PATH}")


def restore_backup() -> bool:
    if not BACKUP_PATH.is_file():
        return False
    shutil.copy2(BACKUP_PATH, CONFIG_PATH)
    print(f"[回滚] 已恢复 {BACKUP_PATH}")
    restart_mihomo()
    return True


def restart_mihomo() -> bool:
    try:
        proc = subprocess.run(
            ["systemctl", "restart", MIHOMO_SERVICE],
            capture_output=True,
            text=True,
            timeout=30,
        )
        if proc.returncode != 0:
            print(f"[重启失败] systemctl: {proc.stderr.strip() or proc.stdout.strip()}")
            return False
        time.sleep(2)
        print(f"[重启] {MIHOMO_SERVICE} 服务已重启")
        return True
    except Exception as e:
        print(f"[重启失败] {e}")
        return False


def validate_mihomo(data: dict) -> tuple[bool, list[str]]:
    """应用后自检：控制器、Web UI、延迟测试。"""
    issues: list[str] = []

    try:
        r = requests.get(f"{CONTROLLER}/", timeout=5)
        if r.status_code != 200:
            issues.append(f"控制器 HTTP {r.status_code}")
    except Exception as e:
        issues.append(f"控制器不可达: {e}")
        return False, issues

    if data.get("external-ui"):
        try:
            r = requests.get(f"{CONTROLLER}/ui/", timeout=5)
            if r.status_code != 200:
                issues.append(f"Web UI /ui/ HTTP {r.status_code}")
        except Exception as e:
            issues.append(f"Web UI 不可达: {e}")

    # 抽一个节点做延迟测试（失败仅警告，不阻断更新）
    proxies = data.get("proxies") or []
    if proxies:
        name = proxies[0].get("name", "")
        if name:
            try:
                import urllib.parse

                enc = urllib.parse.quote(name)
                test_url = urllib.parse.quote(DELAY_TEST_URL)
                r = requests.get(
                    f"{CONTROLLER}/proxies/{enc}/delay?timeout=10000&url={test_url}",
                    timeout=15,
                )
                if r.status_code == 200 and "delay" in r.text:
                    print(f"[校验] 延迟测试 OK ({name[:20]}…): {r.text.strip()}")
                else:
                    issues.append(f"延迟测试异常: {r.text.strip()[:120]}")
            except Exception as e:
                issues.append(f"延迟测试失败: {e}")

    return len(issues) == 0, issues


def apply_subscription(urls: list[str] | None = None, dry_run: bool = False) -> int:
    urls = urls or SUBSCRIPTION_URLS
    result = fetch_subscription_with_fallback(urls)
    if result is None:
        return 1

    data, _body = result
    overrides = load_overrides()
    if overrides:
        print(f"[合并] 本地覆盖项: {', '.join(overrides.keys())}")
    else:
        print("[警告] 未找到 local-overrides.yaml，tun/dns/UI 可能被订阅覆盖")

    merged = merge_config(data, overrides, load_old_config())

    for w in validate_dns_for_fake_ip(merged):
        print(f"[警告] {w}")

    print(
        f"[结果] allow-lan={merged.get('allow-lan')}, "
        f"log-level={merged.get('log-level')}, "
        f"external-ui={merged.get('external-ui')}, "
        f"tun={((merged.get('tun') or {}).get('enable'))}, "
        f"dns.fake-ip-filter={len((merged.get('dns') or {}).get('fake-ip-filter') or [])} 条"
    )

    if dry_run:
        print("[dry-run] 跳过写入与重启")
        return 0

    write_config(merged)

    if not restart_mihomo():
        print("[错误] mihomo 重启失败，尝试回滚...")
        restore_backup()
        return 2

    ok, issues = validate_mihomo(merged)
    if not ok:
        for msg in issues:
            print(f"[校验失败] {msg}")
        print("[错误] 应用后校验未通过，回滚配置...")
        if restore_backup():
            print("[提示] 已回滚到更新前配置")
        return 3

    mark_updated()
    print("[完成] 订阅已更新并生效")
    print(f"[访问] Web UI: http://127.0.0.1:9090/ui/")
    return 0


def mark_updated() -> None:
    LAST_UPDATE_PATH.write_text(
        str(int(datetime.now(timezone.utc).timestamp())),
        encoding="utf-8",
    )


def main() -> int:
    parser = argparse.ArgumentParser(description="拉取并应用 mihomo 订阅")
    parser.add_argument(
        "--fetch-only",
        action="store_true",
        help="仅下载到 subscription_config.yaml，不写入 /etc/mihomo",
    )
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="拉取并合并配置，不写入、不重启",
    )
    parser.add_argument(
        "--url",
        action="append",
        dest="urls",
        help="订阅 URL（可多次指定；默认按内置列表回退）",
    )
    args = parser.parse_args()

    urls = args.urls if args.urls else SUBSCRIPTION_URLS

    if args.fetch_only:
        result = fetch_subscription_with_fallback(urls)
        if result is None:
            return 1
        _data, body = result
        FETCH_ONLY_OUTPUT.write_text(body, encoding="utf-8")
        print(f"[完成] 已保存到 {FETCH_ONLY_OUTPUT}")
        return 0

    return apply_subscription(urls, dry_run=args.dry_run)


if __name__ == "__main__":
    sys.exit(main())
