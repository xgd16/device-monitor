import { useCallback, useEffect, useMemo, useState } from 'react';
import type { ReactNode } from 'react';
import { Button, Card, Chip, Switch } from '@heroui/react';
import {
  RiAlertLine,
  RiCheckboxCircleLine,
  RiDeleteBinLine,
  RiInformationLine,
  RiPlayLine,
  RiRefreshLine,
  RiSendPlaneLine,
  RiStopLine,
} from '@remixicon/react';
import { Band, Panel } from './Panel';
import {
  fetchNfcEmulate,
  fetchNfcEvents,
  fetchNfcStatus,
  fetchNfcTags,
  rawNfcExchange,
  readNfcTag,
  setNfcEmulate,
  setNfcListen,
  setNfcPoll,
  setNfcPower,
  writeNfcTag,
} from '../api';
import type {
  NfcEmulStatus,
  NfcEvent,
  NfcNdefRecord,
  NfcPage,
  NfcRawResult,
  NfcReadResult,
  NfcStatus,
  NfcTag,
  NfcWriteResult,
} from '../types';

// ── 常量 ──

/** 协议字符串 → 中文技术名（与后端字符串一一对应） */
const PROTOCOL_LABELS: Record<string, string> = {
  jewel: 'Jewel / Topaz',
  mifare: 'MIFARE Classic',
  felica: 'FeliCa',
  iso14443a: 'ISO 14443-A',
  nfc_dep: 'NFC-DEP 点对点',
  iso14443b: 'ISO 14443-B',
  iso15693: 'ISO 15693 (NFC-V)',
};

const protocolLabel = (p: string) => PROTOCOL_LABELS[p] ?? p;

/** 轮询可选协议（设备不支持 iso15693） */
const POLL_PROTOCOLS = ['iso14443a', 'mifare', 'jewel', 'iso14443b', 'felica', 'nfc_dep'] as const;
type PollProtocol = (typeof POLL_PROTOCOLS)[number];

/** 能力清单：key + 中文名 + 说明 */
const CAP_ITEMS = [
  ['discover', '发现', '轮询发现卡片'],
  ['read', '读取', '读 NDEF / 原始页'],
  ['write', '写入', '写 NDEF 到卡片'],
  ['emulate', '监听', 'NFC-DEP 点对点监听'],
  ['emulate_uid', '模拟 UID', '模拟成一张卡片'],
] as const;

const WRITE_KINDS = [
  ['text', '文本'],
  ['uri', 'URI'],
  ['raw', '原始十六进制'],
] as const;
type WriteKind = (typeof WRITE_KINDS)[number][0];

/** MIFARE Classic 克隆边界：门禁卡识别依赖 MIFARE Classic 协议 + 扇区数据，模拟卡是 ISO-DEP T4T */
const MIFARE_EMUL_WARN =
  '该卡是 MIFARE Classic：这里只能克隆 UID。门禁卡识别依赖 MIFARE Classic 协议（SAK 08 + CRYPTO1 扇区认证），本模拟是 ISO-DEP T4T 卡，手机钱包/门禁读头不会把它识别为门禁卡。';

/** MIFARE Classic 卡带入模拟表单时返回警告文案 */
const mifareWarn = (t: NfcTag): string | undefined =>
  (t.best_protocol || t.protocol) === 'mifare' ? MIFARE_EMUL_WARN : undefined;

const REFRESH_MS = 2000;

// ── 工具 ──

/** 从 axios 错误里抽出后端 error 文案 */
function errText(e: unknown, fallback: string): string {
  const msg = (e as { response?: { data?: { error?: string } } })?.response?.data?.error;
  if (msg) return msg;
  const code = (e as { code?: string })?.code;
  if (code === 'ERR_NETWORK' || code === 'ECONNABORTED') return '无法连接后端服务';
  return fallback;
}

/** 十六进制规范化：允许空格/冒号/大小写，输出偶数长度的小写 hex */
function normalizeHex(input: string): { ok: true; hex: string } | { ok: false; error: string } {
  const cleaned = input.replace(/[\s:]/g, '').toLowerCase();
  if (!cleaned) return { ok: false, error: '请输入十六进制数据' };
  if (!/^[0-9a-f]+$/.test(cleaned)) {
    return { ok: false, error: '含非法字符，仅允许 0-9 / a-f 与空格、冒号分隔' };
  }
  if (cleaned.length % 2 !== 0) return { ok: false, error: '十六进制长度必须为偶数' };
  return { ok: true, hex: cleaned };
}

/** UID 以冒号分隔展示 */
function formatUid(uid: string): string {
  const hex = uid.replace(/[\s:]/g, '').toUpperCase();
  if (!hex) return '—';
  return hex.match(/.{1,2}/g)?.join(':') ?? hex;
}

/** 事件时间归一化：兼容 ISO 字符串与秒/毫秒时间戳字符串 */
function eventTime(at: string): number {
  const n = Number(at);
  if (at.trim() !== '' && Number.isFinite(n)) return n < 1e12 ? n * 1000 : n;
  return new Date(at).getTime();
}

function fmtTime(at: string): string {
  const t = eventTime(at);
  if (!Number.isFinite(t)) return at;
  return new Date(t).toLocaleTimeString('zh-CN', { hour12: false });
}

/** 卡片在列表里的稳定 key */
const tagKey = (t: NfcTag) => `${t.index}:${t.uid}`;

// ── 内部小组件 ──

/** 面板内的次级说明文字 */
function Note({ children }: { children: ReactNode }) {
  return <p className="font-mono text-[10px] leading-relaxed opacity-35">{children}</p>;
}

/** 「标签 — 值」行 */
function Row({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="flex items-start justify-between gap-3 border-b border-default-200/60 py-1.5 last:border-b-0">
      <span className="shrink-0 font-mono text-[10px] opacity-45">{label}</span>
      <span className="min-w-0 flex-1 break-all text-right font-mono text-[11px]">{children}</span>
    </div>
  );
}

/** 提示条：danger / warning / accent 三档 */
function Alert({
  tone = 'danger',
  children,
}: {
  tone?: 'danger' | 'warning' | 'accent';
  children: ReactNode;
}) {
  const cls =
    tone === 'danger'
      ? 'border-danger/40 bg-danger-50 text-danger'
      : tone === 'warning'
        ? 'border-warning/40 bg-warning-50 text-warning'
        : 'border-accent/40 bg-accent-50 text-accent';
  const Icon = tone === 'accent' ? RiInformationLine : RiAlertLine;
  return (
    <div
      className={`flex items-start gap-2 rounded-lg border px-2.5 py-2 text-[11px] leading-relaxed ${cls}`}
    >
      <Icon className="mt-px size-3.5 shrink-0" aria-hidden />
      <div className="min-w-0 flex-1">{children}</div>
    </div>
  );
}

/** 二次确认弹窗 */
function ConfirmModal({
  open,
  title,
  confirmLabel,
  busy,
  onClose,
  onConfirm,
  children,
}: {
  open: boolean;
  title: string;
  confirmLabel: string;
  busy?: boolean;
  onClose: () => void;
  onConfirm: () => void;
  children: ReactNode;
}) {
  if (!open) return null;
  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/40 p-4"
      onClick={onClose}
    >
      <div onClick={(e) => e.stopPropagation()}>
        <Card className="w-full max-w-md border border-default-200 p-4 shadow-lg">
          <h3 className="mb-3 text-sm font-semibold">{title}</h3>
          {children}
          <div className="mt-4 flex justify-end gap-2">
            <Button size="sm" variant="ghost" isDisabled={busy} onPress={onClose}>
              取消
            </Button>
            <Button size="sm" variant="danger" isDisabled={busy} onPress={onConfirm}>
              {confirmLabel}
            </Button>
          </div>
        </Card>
      </div>
    </div>
  );
}

/** NDEF 单条记录 */
function RecordView({ rec }: { rec: NfcNdefRecord }) {
  const label = rec.kind === 'text' ? '文本' : rec.kind === 'uri' ? 'URI' : '原始';
  const color = rec.kind === 'text' ? 'accent' : rec.kind === 'uri' ? 'success' : 'default';
  return (
    <div className="dm-inset flex flex-col gap-1 px-2.5 py-2">
      <div className="flex items-center gap-2">
        <Chip size="sm" variant="secondary" color={color}>
          {label}
        </Chip>
        <span className="truncate font-mono text-[10px] opacity-40">
          TNF {rec.tnf} · {rec.type || '—'}
        </span>
      </div>
      {rec.kind === 'text' && rec.text != null && (
        <p className="break-all font-mono text-xs">{rec.text}</p>
      )}
      {rec.kind === 'uri' && rec.uri != null && (
        <a
          href={rec.uri}
          target="_blank"
          rel="noreferrer"
          className="break-all font-mono text-xs text-accent hover:underline"
        >
          {rec.uri}
        </a>
      )}
      <span className="break-all font-mono text-[9px] opacity-35">
        payload {rec.payload_hex || '—'}
      </span>
    </div>
  );
}

/** 原始页转储表 */
function PagesTable({ pages }: { pages: NfcPage[] }) {
  return (
    <div className="dm-inset max-h-56 overflow-auto">
      <table className="w-full border-collapse font-mono text-[10px]">
        <thead className="dm-table-head sticky top-0">
          <tr>
            <th className="px-2 py-1 text-left font-medium opacity-55">页</th>
            <th className="px-2 py-1 text-left font-medium opacity-55">HEX</th>
            <th className="px-2 py-1 text-left font-medium opacity-55">ASCII</th>
          </tr>
        </thead>
        <tbody>
          {pages.map((p) => (
            <tr key={p.page} className="dm-row">
              <td className="px-2 py-1 tabular-nums opacity-45">{p.page}</td>
              <td className="whitespace-pre px-2 py-1">{p.hex}</td>
              <td className="whitespace-pre px-2 py-1 opacity-70">{p.ascii}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

// ── 页面 ──

export function NfcPanel() {
  const [status, setStatus] = useState<NfcStatus | null>(null);
  const [statusError, setStatusError] = useState<string | null>(null);
  const [tags, setTags] = useState<NfcTag[]>([]);
  const [events, setEvents] = useState<NfcEvent[]>([]);
  const [clearedAt, setClearedAt] = useState<number | null>(null);
  const [busy, setBusy] = useState<string | null>(null);

  const [pollProtocols, setPollProtocols] = useState<PollProtocol[]>([...POLL_PROTOCOLS]);
  const [selectedKey, setSelectedKey] = useState<string | null>(null);

  const [startPage, setStartPage] = useState('4');
  const [pageCount, setPageCount] = useState('16');
  const [readResult, setReadResult] = useState<NfcReadResult | null>(null);
  const [readError, setReadError] = useState<string | null>(null);

  const [writeKind, setWriteKind] = useState<WriteKind>('text');
  const [writeValue, setWriteValue] = useState('');
  const [writeLang, setWriteLang] = useState('zh');
  const [writeError, setWriteError] = useState<string | null>(null);
  const [writeResult, setWriteResult] = useState<NfcWriteResult | null>(null);
  const [confirmOpen, setConfirmOpen] = useState(false);

  const [emul, setEmul] = useState<NfcEmulStatus | null>(null);
  const [emulUid, setEmulUid] = useState('db4c1b5a');
  const [emulKind, setEmulKind] = useState<'text' | 'uri' | 'message'>('text');
  const [emulValue, setEmulValue] = useState('Hello NFC');
  const [emulLang, setEmulLang] = useState('zh');
  const [emulRaw, setEmulRaw] = useState('');
  const [emulWarn, setEmulWarn] = useState<string | null>(null);
  const [emulError, setEmulError] = useState<string | null>(null);

  const [txHex, setTxHex] = useState('30 04');
  const [txError, setTxError] = useState<string | null>(null);
  const [rawResult, setRawResult] = useState<NfcRawResult | null>(null);

  const refreshStatus = useCallback(() => {
    fetchNfcStatus()
      .then((d) => {
        setStatus(d);
        setStatusError(null);
      })
      .catch((e) => setStatusError(errText(e, 'NFC 状态获取失败')));
  }, []);

  const refreshEmulate = useCallback(() => {
    fetchNfcEmulate()
      .then(setEmul)
      .catch(() => {
        /* 模拟状态失败静默，不影响其它卡片 */
      });
  }, []);

  const refreshTags = useCallback(() => {
    fetchNfcTags()
      .then(setTags)
      .catch(() => {
        /* 卡片列表失败不打断界面，状态卡会给出原因 */
      });
  }, []);

  useEffect(() => {
    refreshStatus();
    refreshTags();
    refreshEmulate();
    const t = setInterval(() => {
      refreshStatus();
      refreshTags();
      refreshEmulate();
    }, REFRESH_MS);
    return () => clearInterval(t);
  }, [refreshStatus, refreshTags, refreshEmulate]);

  useEffect(() => {
    const load = () =>
      fetchNfcEvents(50)
        .then(setEvents)
        .catch(() => {
          /* 事件拉取失败静默，避免刷屏 */
        });
    load();
    const t = setInterval(load, REFRESH_MS);
    return () => clearInterval(t);
  }, []);

  // 选中卡片以 index:uid 为键；卡片消失时 selectedTag 自然变为 null，无需在 effect 里改状态
  const selectedTag = useMemo(
    () => tags.find((t) => tagKey(t) === selectedKey) ?? null,
    [tags, selectedKey],
  );

  const available = status?.available ?? false;
  const polling = status?.polling ?? false;
  const listening = status?.listening ?? false;
  const caps = status?.capabilities ?? null;
  const canDiscover = available && (caps?.discover.ok ?? false);
  const canRead = available && (caps?.read.ok ?? false);
  const canWrite = available && (caps?.write.ok ?? false);
  const canEmulate = available && (caps?.emulate.ok ?? false);
  const emulRunning = emul?.running ?? false;

  const visibleEvents = useMemo(() => {
    const list = clearedAt == null ? events : events.filter((e) => eventTime(e.at) > clearedAt);
    return [...list].sort((a, b) => eventTime(b.at) - eventTime(a.at));
  }, [events, clearedAt]);

  const targetProtocol = selectedTag ? selectedTag.best_protocol || selectedTag.protocol : '';

  // ── 操作 ──

  const handlePower = async (up: boolean) => {
    setBusy('power');
    setStatusError(null);
    try {
      await setNfcPower(up);
      refreshStatus();
    } catch (e) {
      setStatusError(errText(e, up ? '上电失败' : '下电失败'));
    } finally {
      setBusy(null);
    }
  };

  const handlePoll = async (action: 'start' | 'stop') => {
    setBusy(`poll-${action}`);
    setStatusError(null);
    try {
      const res = await setNfcPoll(action, action === 'start' ? pollProtocols : undefined);
      if (action === 'start') {
        // 以服务端确认的协议集回填开关（为空时保留本地选择）
        const adopted = POLL_PROTOCOLS.filter((p) => res.protocols.includes(p));
        if (adopted.length > 0) setPollProtocols(adopted);
      }
      refreshStatus();
      refreshTags();
    } catch (e) {
      setStatusError(errText(e, '轮询操作失败'));
    } finally {
      setBusy(null);
    }
  };

  const toggleProtocol = (p: PollProtocol) => {
    setPollProtocols((prev) =>
      POLL_PROTOCOLS.filter((x) => (x === p ? !prev.includes(x) : prev.includes(x))),
    );
  };

  const handleRead = async (mode: 'ndef' | 'pages') => {
    if (!selectedTag) {
      setReadError('请先在卡片列表中选中一张卡片');
      return;
    }
    if (mode === 'pages') {
      const s = Number(startPage);
      const c = Number(pageCount);
      if (!Number.isInteger(s) || s < 0) {
        setReadError('起始页必须是非负整数');
        return;
      }
      if (!Number.isInteger(c) || c < 1 || c > 256) {
        setReadError('页数需在 1–256 之间');
        return;
      }
    }
    setBusy(`read-${mode}`);
    setReadError(null);
    try {
      const res = await readNfcTag({
        target_index: selectedTag.index,
        protocol: targetProtocol,
        mode,
        ...(mode === 'pages'
          ? { start_page: Number(startPage), page_count: Number(pageCount) }
          : {}),
      });
      setReadResult((prev) =>
        prev && prev.uid === res.uid
          ? {
              ...res,
              ndef: mode === 'ndef' ? res.ndef : prev.ndef,
              pages: mode === 'pages' ? res.pages : prev.pages,
            }
          : res,
      );
    } catch (e) {
      setReadError(errText(e, '读取失败'));
    } finally {
      setBusy(null);
    }
  };

  /** 写入前校验；通过后弹二次确认 */
  const handleWriteClick = () => {
    setWriteError(null);
    setWriteResult(null);
    if (!selectedTag) {
      setWriteError('请先在卡片列表中选中一张卡片');
      return;
    }
    if (writeKind === 'raw') {
      const r = normalizeHex(writeValue);
      if (!r.ok) {
        setWriteError(r.error);
        return;
      }
    } else if (!writeValue.trim()) {
      setWriteError(writeKind === 'uri' ? '请输入 URI' : '请输入文本内容');
      return;
    }
    setConfirmOpen(true);
  };

  const handleWrite = async () => {
    if (!selectedTag) return;
    setConfirmOpen(false);
    setBusy('write');
    setWriteError(null);
    setWriteResult(null);
    const params: {
      target_index: number;
      protocol: string;
      kind: WriteKind;
      value: string;
      lang?: string;
      raw_hex?: string;
    } = {
      target_index: selectedTag.index,
      protocol: targetProtocol,
      kind: writeKind,
      value: writeValue,
    };
    if (writeKind === 'text') params.lang = writeLang.trim() || 'zh';
    if (writeKind === 'raw') {
      const r = normalizeHex(writeValue);
      const hex = r.ok ? r.hex : '';
      params.value = hex;
      params.raw_hex = hex;
    }
    try {
      setWriteResult(await writeNfcTag(params));
      refreshTags();
    } catch (e) {
      setWriteError(errText(e, '写入失败'));
    } finally {
      setBusy(null);
    }
  };

  const handleRawSend = async () => {
    setTxError(null);
    if (!selectedTag) {
      setTxError('请先在卡片列表中选中一张卡片');
      return;
    }
    const r = normalizeHex(txHex);
    if (!r.ok) {
      setTxError(r.error);
      return;
    }
    setBusy('raw');
    setRawResult(null);
    try {
      setRawResult(
        await rawNfcExchange({
          target_index: selectedTag.index,
          protocol: targetProtocol,
          tx_hex: r.hex,
        }),
      );
    } catch (e) {
      setTxError(errText(e, '原始帧发送失败'));
    } finally {
      setBusy(null);
    }
  };

  const handleEmulate = async (action: 'start' | 'stop') => {
    setBusy(action === 'start' ? 'emul-start' : 'emul-stop');
    setEmulError(null);
    try {
      const d =
        action === 'start'
          ? await setNfcEmulate({
              action,
              uid: emulUid,
              kind: emulKind,
              value: emulValue,
              lang: emulLang,
              raw_hex: emulKind === 'message' ? emulRaw : '',
            })
          : await setNfcEmulate({ action });
      setEmul(d);
      refreshStatus();
    } catch (e) {
      setEmulError(errText(e, action === 'start' ? '启动模拟失败' : '停止模拟失败'));
    } finally {
      setBusy(null);
      refreshEmulate();
    }
  };

  /** 一键把发现/读到的卡片带入模拟表单：UID 必带；读过 NDEF 则整条克隆 */
  const emulateFrom = (uid: string, ndefRaw?: string, warn?: string) => {
    setEmulUid(uid.replace(/[\s:]/g, ''));
    if (ndefRaw) {
      setEmulKind('message');
      setEmulRaw(ndefRaw);
    }
    setEmulWarn(warn ?? null);
    setEmulError(null);
    document
      .getElementById('nfc-emul-uid')
      ?.scrollIntoView({ behavior: 'smooth', block: 'center' });
  };

  const handleListen = async (action: 'start' | 'stop') => {
    setBusy(`listen-${action}`);
    setStatusError(null);
    try {
      await setNfcListen(action);
      refreshStatus();
    } catch (e) {
      setStatusError(errText(e, '监听操作失败'));
    } finally {
      setBusy(null);
    }
  };

  const txPreview = normalizeHex(txHex);
  // raw 写入模式的十六进制预览（仅用于就地提示）
  const writeHexPreview = writeKind === 'raw' ? normalizeHex(writeValue) : null;

  return (
    <div className="mx-auto flex w-full max-w-[1760px] flex-col gap-4 p-4 lg:gap-5 lg:p-5">
      <Band index={1} title="硬件与轮询" cols={2}>
        {/* 硬件状态 */}
        <Panel
          label="硬件状态"
          index={1}
          hint={
            <>
              <Chip size="sm" variant="secondary" color={available ? 'success' : 'danger'}>
                {available ? '就绪' : '不可用'}
              </Chip>
              {polling && (
                <Chip size="sm" variant="secondary" color="accent">
                  轮询中
                </Chip>
              )}
              {listening && (
                <Chip size="sm" variant="secondary" color="warning">
                  监听中
                </Chip>
              )}
              <Button
                size="sm"
                variant="ghost"
                isDisabled={busy === 'power'}
                onPress={refreshStatus}
                className="h-6 min-w-0 px-1.5 font-mono text-[10px]"
              >
                <RiRefreshLine className="size-3.5" aria-hidden />
                刷新
              </Button>
            </>
          }
        >
          {statusError && <Alert>{statusError}</Alert>}

          {!status ? (
            <span className="font-mono text-sm opacity-30">正在读取 NFC 状态...</span>
          ) : (
            <>
              {!status.available && (
                <Alert tone="warning">{status.message || 'NFC 硬件不可用'}</Alert>
              )}

              <div className="dm-inset px-2.5 py-1">
                <Row label="平台">{status.platform || '—'}</Row>
                <Row label="设备">
                  {status.device ? `${status.device.name} (#${status.device.index})` : '未发现'}
                </Row>
                <Row label="上电">
                  <Chip
                    size="sm"
                    variant="secondary"
                    color={status.device?.powered ? 'success' : 'default'}
                  >
                    {status.device?.powered ? '已上电' : '未上电'}
                  </Chip>
                </Row>
                <Row label="rfkill">
                  {status.device == null ? (
                    '—'
                  ) : status.device.rfkill_hard_blocked ? (
                    <span className="text-danger">硬件锁</span>
                  ) : status.device.rfkill_soft_blocked ? (
                    <span className="text-warning">软件锁</span>
                  ) : (
                    '未锁定'
                  )}
                </Row>
                <Row label="协议芯片">
                  {status.device && status.device.protocols.length > 0
                    ? status.device.protocols.map(protocolLabel).join(' · ')
                    : '—'}
                </Row>
                <Row label="安全单元">
                  {status.secure_elements.length > 0
                    ? status.secure_elements
                        .map((se) => `#${se.index} (type ${se.se_type})`)
                        .join(' · ')
                    : '无'}
                </Row>
                <Row label="在线卡片">{status.target_count}</Row>
                <Row label="最近操作">
                  {status.last_op ?? '—'}
                  {status.last_error ? ` · ${status.last_error}` : ''}
                </Row>
              </div>

              <div className="flex gap-2">
                <Button
                  size="sm"
                  variant="secondary"
                  isDisabled={!available || busy === 'power' || status.device?.powered === true}
                  onPress={() => handlePower(true)}
                  className="flex-1 font-mono text-xs"
                >
                  {busy === 'power' ? '处理中...' : '上电'}
                </Button>
                <Button
                  size="sm"
                  variant="secondary"
                  isDisabled={!available || busy === 'power' || status.device?.powered !== true}
                  onPress={() => handlePower(false)}
                  className="flex-1 font-mono text-xs"
                >
                  下电
                </Button>
              </div>

              <div className="grid grid-cols-1 gap-1.5 sm:grid-cols-2">
                {CAP_ITEMS.map(([key, name, desc]) => {
                  const cap = caps?.[key];
                  const ok = cap?.ok ?? false;
                  return (
                    <div
                      key={key}
                      className="dm-inset flex items-start gap-1.5 px-2 py-1.5"
                      title={cap?.reason || desc}
                    >
                      {ok ? (
                        <RiCheckboxCircleLine
                          className="mt-px size-3.5 shrink-0 text-success"
                          aria-hidden
                        />
                      ) : (
                        <RiAlertLine className="mt-px size-3.5 shrink-0 text-warning" aria-hidden />
                      )}
                      <div className="min-w-0 flex-1">
                        <div className="font-mono text-[11px]">
                          {name}
                          <span className="ml-1 opacity-40">{ok ? '可用' : '不可用'}</span>
                        </div>
                        {!ok && cap?.reason && (
                          <div className="break-all font-mono text-[9px] leading-relaxed opacity-40">
                            {cap.reason}
                          </div>
                        )}
                      </div>
                    </div>
                  );
                })}
              </div>
            </>
          )}

          <Note>设备不可用时所有操作都会被禁用；状态每 2 秒自动刷新，接口异常只在本卡内提示。</Note>
        </Panel>

        {/* 卡片发现 */}
        <Panel
          label="卡片发现"
          index={2}
          hint={
            <>
              <span>发现 {tags.length} 张</span>
              {selectedTag && <span className="text-accent">已选 #{selectedTag.index}</span>}
            </>
          }
        >
          <div className="grid grid-cols-2 gap-x-3 gap-y-1.5 sm:grid-cols-3">
            {POLL_PROTOCOLS.map((p) => (
              <Switch
                key={p}
                size="sm"
                isSelected={pollProtocols.includes(p)}
                isDisabled={!canDiscover || polling || busy !== null}
                onChange={() => toggleProtocol(p)}
              >
                <span className="font-mono text-[11px]">{protocolLabel(p)}</span>
              </Switch>
            ))}
          </div>

          <div className="flex gap-2">
            <Button
              size="sm"
              variant="secondary"
              isDisabled={
                !canDiscover || polling || pollProtocols.length === 0 || busy === 'poll-start'
              }
              onPress={() => handlePoll('start')}
              className="flex-1 font-mono text-xs"
            >
              <RiPlayLine className="size-3.5" aria-hidden />
              {busy === 'poll-start' ? '启动中...' : '开始轮询'}
            </Button>
            <Button
              size="sm"
              variant={polling ? 'danger' : 'ghost'}
              isDisabled={!available || !polling || busy === 'poll-stop'}
              onPress={() => handlePoll('stop')}
              className="flex-1 font-mono text-xs"
            >
              <RiStopLine className="size-3.5" aria-hidden />
              停止轮询
            </Button>
          </div>

          {tags.length === 0 ? (
            <span className="font-mono text-[11px] leading-relaxed opacity-35">
              {polling ? '轮询中：请把卡片贴近设备背面 NFC 区域' : '未发现卡片（先开始轮询）'}
            </span>
          ) : (
            <div className="dm-inset max-h-64 overflow-y-auto">
              {tags.map((t) => {
                const key = tagKey(t);
                const active = key === selectedKey;
                // 同一张卡读取过 NDEF 时，「模拟此卡」连内容一起克隆
                const ndefRaw =
                  readResult?.uid === t.uid ? (readResult.ndef?.raw_hex ?? undefined) : undefined;
                return (
                  <div
                    key={key}
                    className={`dm-row flex items-center gap-2 px-2.5 py-2 transition-colors ${
                      active ? 'dm-table-row-selected' : 'hover:bg-default-100'
                    }`}
                  >
                    <button
                      type="button"
                      onClick={() => setSelectedKey(key)}
                      aria-pressed={active}
                      className="flex min-w-0 flex-1 items-center gap-2 text-left"
                    >
                      <span
                        className={`size-1.5 shrink-0 rounded-full ${t.lost ? 'bg-default-300' : 'bg-success'}`}
                        aria-hidden
                      />
                      <span className="min-w-0 flex-1">
                        <span className="block truncate font-mono text-[11px]">
                          {formatUid(t.uid)}
                        </span>
                        <span className="block truncate font-mono text-[9px] opacity-40">
                          #{t.index} · {protocolLabel(t.best_protocol || t.protocol)} ·{' '}
                          {t.technology} · ATQA {t.atqa || '—'} · SAK {t.sak ?? '—'}
                        </span>
                      </span>
                    </button>
                    {t.lost && (
                      <Chip size="sm" variant="secondary" color="default">
                        已离开
                      </Chip>
                    )}
                    <Button
                      size="sm"
                      variant="ghost"
                      isDisabled={emulRunning || busy !== null}
                      onPress={() => emulateFrom(t.uid, ndefRaw, mifareWarn(t))}
                      className="shrink-0 font-mono text-[10px]"
                    >
                      模拟此卡
                    </Button>
                  </div>
                );
              })}
            </div>
          )}

          <Note>
            可选协议：ISO 14443-A / MIFARE Classic / Jewel / ISO 14443-B / FeliCa /
            NFC-DEP（iso15693 设备不支持）。选中一张卡片后，读取、写入与原始帧都作用于它；
            「模拟此卡」把 UID（读取过则连同 NDEF 内容）带入卡模拟面板。
          </Note>
        </Panel>
      </Band>

      <Band index={2} title="读取与写入" cols={2}>
        {/* 读取 */}
        <Panel
          label="读取卡片"
          index={3}
          hint={
            selectedTag ? (
              <span className="text-accent">{formatUid(selectedTag.uid)}</span>
            ) : (
              <span className="opacity-50">未选择卡片</span>
            )
          }
        >
          <div className="flex flex-wrap items-end gap-2">
            <label className="flex flex-col gap-1">
              <span className="font-mono text-[10px] opacity-45">起始页</span>
              <input
                className="dm-input w-20 py-1 font-mono text-xs"
                inputMode="numeric"
                value={startPage}
                onChange={(e) => setStartPage(e.target.value)}
                disabled={!canRead || busy !== null}
              />
            </label>
            <label className="flex flex-col gap-1">
              <span className="font-mono text-[10px] opacity-45">页数</span>
              <input
                className="dm-input w-20 py-1 font-mono text-xs"
                inputMode="numeric"
                value={pageCount}
                onChange={(e) => setPageCount(e.target.value)}
                disabled={!canRead || busy !== null}
              />
            </label>
            <div className="flex flex-1 flex-wrap gap-2">
              <Button
                size="sm"
                variant="secondary"
                isDisabled={!canRead || !selectedTag || busy === 'read-ndef'}
                onPress={() => handleRead('ndef')}
                className="flex-1 font-mono text-xs"
              >
                {busy === 'read-ndef' ? '读取中...' : '读取 NDEF'}
              </Button>
              <Button
                size="sm"
                variant="secondary"
                isDisabled={!canRead || !selectedTag || busy === 'read-pages'}
                onPress={() => handleRead('pages')}
                className="flex-1 font-mono text-xs"
              >
                {busy === 'read-pages' ? '读取中...' : '读取原始页'}
              </Button>
            </div>
          </div>

          {!canRead && caps?.read.reason && <Alert tone="warning">{caps.read.reason}</Alert>}
          {readError && <Alert>{readError}</Alert>}

          {readResult ? (
            <div className="flex flex-col gap-2">
              <div className="dm-inset px-2.5 py-1">
                <Row label="UID">
                  <span className="text-accent">{formatUid(readResult.uid)}</span>
                </Row>
                <Row label="技术类型">{readResult.technology || '—'}</Row>
                <Row label="NDEF 大小">{readResult.ndef ? `${readResult.ndef.size} B` : '—'}</Row>
              </div>
              {readResult.note && <Note>{readResult.note}</Note>}

              {readResult.ndef &&
                (readResult.ndef.records.length === 0 ? (
                  <span className="font-mono text-[11px] opacity-35">NDEF 消息为空</span>
                ) : (
                  <div className="flex flex-col gap-1.5">
                    {readResult.ndef.records.map((rec, i) => (
                      <RecordView key={`${rec.tnf}-${rec.type}-${i}`} rec={rec} />
                    ))}
                  </div>
                ))}

              {readResult.ndef && readResult.ndef.raw_hex && (
                <div className="dm-inset px-2.5 py-2">
                  <div className="mb-1 font-mono text-[10px] opacity-45">
                    原始 NDEF 十六进制（{readResult.ndef.size} B）
                  </div>
                  <p className="max-h-24 overflow-auto break-all font-mono text-[10px] opacity-80">
                    {readResult.ndef.raw_hex}
                  </p>
                  <Button
                    size="sm"
                    variant="secondary"
                    isDisabled={emulRunning || busy !== null}
                    onPress={() => {
                      const raw = readResult?.ndef?.raw_hex;
                      if (raw && readResult)
                        emulateFrom(
                          readResult.uid,
                          raw,
                          (readResult.technology ?? '').includes('MIFARE')
                            ? MIFARE_EMUL_WARN
                            : undefined,
                        );
                    }}
                    className="mt-2 font-mono text-xs"
                  >
                    用此内容模拟（UID + NDEF 克隆）
                  </Button>
                </div>
              )}

              {readResult.pages && readResult.pages.length > 0 && (
                <PagesTable pages={readResult.pages} />
              )}

              {readResult.raw_exchanges.length > 0 && (
                <details className="dm-inset px-2.5 py-2">
                  <summary className="cursor-pointer font-mono text-[10px] opacity-45">
                    原始交换（{readResult.raw_exchanges.length}）
                  </summary>
                  <div className="mt-1.5 flex flex-col gap-1">
                    {readResult.raw_exchanges.map((x, i) => (
                      <div key={i} className="font-mono text-[10px] leading-relaxed">
                        <span className="opacity-40">TX </span>
                        <span className="break-all">{x.tx || '—'}</span>
                        <br />
                        <span className="opacity-40">RX </span>
                        <span className="break-all">{x.rx || '—'}</span>
                      </div>
                    ))}
                  </div>
                </details>
              )}
            </div>
          ) : (
            <span className="font-mono text-[11px] opacity-35">
              选中卡片后读取：NDEF 解析出的文本/URI 直接可读，原始页以 HEX + ASCII 表格展示。
            </span>
          )}

          <Note>
            「读取原始页」适合 NTAG/Ultralight（页 0–3 为厂商区，建议从第 4 页开始）；MIFARE Classic
            无 NDEF 时会返回错误说明。
          </Note>
        </Panel>

        {/* 写入 */}
        <Panel
          label="写入卡片"
          index={4}
          hint={selectedTag ? <span className="text-accent">#{selectedTag.index}</span> : undefined}
        >
          <Alert tone="warning">
            <p className="font-medium">写入会覆盖卡片原有 NDEF 区，且不可撤销。</p>
            <p className="mt-1 opacity-80">
              仅 NTAG / Ultralight 等 Type 2 标签与部分 Type 4 标签支持直接写入；MIFARE Classic
              需要密钥认证，后端会返回对应错误说明。
            </p>
          </Alert>

          {!canWrite && caps?.write.reason && <Alert tone="warning">{caps.write.reason}</Alert>}

          <div className="flex gap-2">
            {WRITE_KINDS.map(([kind, label]) => (
              <Button
                key={kind}
                size="sm"
                variant={writeKind === kind ? 'secondary' : 'ghost'}
                isDisabled={!canWrite || busy === 'write'}
                onPress={() => setWriteKind(kind)}
                className="flex-1 font-mono text-xs"
              >
                {label}
              </Button>
            ))}
          </div>

          {writeKind === 'raw' ? (
            <textarea
              className="dm-input font-mono text-xs"
              rows={3}
              placeholder="例如：03 10 D1 01 0A 54 02 7A 68 ..."
              value={writeValue}
              onChange={(e) => setWriteValue(e.target.value)}
              disabled={!canWrite || busy === 'write'}
            />
          ) : (
            <input
              className="dm-input font-mono text-xs"
              placeholder={writeKind === 'text' ? '要写入的文本内容' : 'https://example.com'}
              value={writeValue}
              onChange={(e) => setWriteValue(e.target.value)}
              disabled={!canWrite || busy === 'write'}
            />
          )}

          <div className="flex flex-wrap items-end gap-2">
            {writeKind === 'text' && (
              <label className="flex flex-col gap-1">
                <span className="font-mono text-[10px] opacity-45">语言码</span>
                <input
                  className="dm-input w-24 py-1 font-mono text-xs"
                  value={writeLang}
                  onChange={(e) => setWriteLang(e.target.value)}
                  disabled={!canWrite || busy === 'write'}
                />
              </label>
            )}
            {writeKind === 'raw' && (
              <span className="font-mono text-[10px] opacity-45">
                {writeHexPreview == null || writeValue.trim() === ''
                  ? '请输入十六进制'
                  : writeHexPreview.ok
                    ? `规范化：${writeHexPreview.hex}`
                    : writeHexPreview.error}
              </span>
            )}
            <Button
              size="sm"
              variant="danger"
              isDisabled={!canWrite || !selectedTag || busy === 'write'}
              onPress={handleWriteClick}
              className="ml-auto font-mono text-xs"
            >
              {busy === 'write' ? '写入中...' : '写入卡片'}
            </Button>
          </div>

          {writeError && <Alert>{writeError}</Alert>}

          {writeResult && (
            <div className="dm-inset px-2.5 py-1">
              <Row label="写入字节">{writeResult.wrote_bytes}</Row>
              <Row label="模板">{writeResult.template || '—'}</Row>
              <Row label="回读校验">
                {writeResult.verify ? `记录 ${writeResult.verify.records} 条` : '未校验'}
              </Row>
              {writeResult.verify?.raw_hex && (
                <Row label="回读十六进制">
                  <span className="opacity-80">{writeResult.verify.raw_hex}</span>
                </Row>
              )}
            </div>
          )}

          <Note>
            写入前会弹窗二次确认；原始十六进制模式下会把空格/冒号与大写统一规范化为小写偶数字节。
          </Note>
        </Panel>
      </Band>

      <Band index={3} title="高级操作" cols={2}>
        {/* 原始帧 */}
        <Panel
          label="原始帧控制台"
          index={5}
          hint={selectedTag ? <span>协议 {protocolLabel(targetProtocol)}</span> : undefined}
        >
          <textarea
            className="dm-input font-mono text-xs"
            rows={2}
            placeholder="十六进制帧，例如：30 04（MIFARE 读块 4）"
            value={txHex}
            onChange={(e) => setTxHex(e.target.value)}
            disabled={!available || busy === 'raw'}
          />
          <div className="flex items-center gap-2">
            <span className="min-w-0 flex-1 truncate font-mono text-[10px] opacity-45">
              {txHex.trim() === ''
                ? '等待输入'
                : txPreview.ok
                  ? `规范化：${txPreview.hex}（${txPreview.hex.length / 2} B）`
                  : txPreview.error}
            </span>
            <Button
              size="sm"
              variant="secondary"
              isDisabled={!available || !selectedTag || busy === 'raw' || !txPreview.ok}
              onPress={handleRawSend}
              className="font-mono text-xs"
            >
              <RiSendPlaneLine className="size-3.5" aria-hidden />
              {busy === 'raw' ? '发送中...' : '发送'}
            </Button>
          </div>

          {txError && <Alert>{txError}</Alert>}

          {rawResult && (
            <div className="dm-inset px-2.5 py-1">
              <Row label="TX">
                <span className="break-all">{rawResult.tx || '—'}</span>
              </Row>
              <Row label="RX">
                <span className="break-all">{rawResult.rx || '—'}</span>
              </Row>
              <Row label="耗时">{rawResult.elapsed_ms} ms</Row>
              {rawResult.error && (
                <Row label="错误">
                  <span className="text-danger">{rawResult.error}</span>
                </Row>
              )}
            </div>
          )}

          <Note>
            用于 MIFARE Classic 等高级操作：原样发送 APDU/帧并回显
            RX。需要先选中卡片并保持与卡片通信。
          </Note>
        </Panel>

        {/* 监听 / 模仿 */}
        <Panel
          label="监听 / 模仿卡"
          index={6}
          hint={
            <Chip size="sm" variant="secondary" color={listening ? 'warning' : 'default'}>
              {listening ? '监听中' : '未监听'}
            </Chip>
          }
        >
          <Alert tone="accent">
            <p className="font-medium">
              这是 NFC-DEP 点对点监听（目标模式）；模拟固定 UID 的卡片请用「UID 卡模拟」面板。
            </p>
            <p className="mt-1 opacity-80">
              监听能力：{caps?.emulate.reason || '状态未知，等待后端返回'}
            </p>
            <p className="opacity-80">
              模拟 UID：{caps?.emulate_uid.reason || '状态未知，等待后端返回'}
            </p>
          </Alert>

          {status?.listen_protocols && status.listen_protocols.length > 0 && (
            <Row label="监听协议">{status.listen_protocols.map(protocolLabel).join(' · ')}</Row>
          )}

          <div className="flex gap-2">
            <Button
              size="sm"
              variant="secondary"
              isDisabled={!canEmulate || listening || busy === 'listen-start'}
              onPress={() => handleListen('start')}
              className="flex-1 font-mono text-xs"
            >
              {busy === 'listen-start' ? '开启中...' : '开启监听'}
            </Button>
            <Button
              size="sm"
              variant={listening ? 'danger' : 'ghost'}
              isDisabled={!available || !listening || busy === 'listen-stop'}
              onPress={() => handleListen('stop')}
              className="flex-1 font-mono text-xs"
            >
              停止监听
            </Button>
          </div>

          <Note>
            开启监听后需要另一台设备（手机/读卡器）以 NFC-DEP 模式靠近；监听期间不能同时轮询。
          </Note>
        </Panel>

        {/* UID 卡模拟（用户态直驱芯片） */}
        <Panel
          label="UID 卡模拟"
          index={7}
          hint={
            <Chip
              size="sm"
              variant="secondary"
              color={emulRunning ? (emul?.active ? 'success' : 'accent') : 'default'}
            >
              {emulRunning ? (emul?.active ? '读卡器已激活' : '模拟中') : '未模拟'}
            </Chip>
          }
        >
          <Alert tone="accent">
            <p className="font-medium">
              模拟一张 ISO14443-A 卡：自定义 UID + T4T NDEF 内容；也可「模拟此卡」克隆发现过的卡片。
            </p>
            <p className="mt-1 opacity-80">
              {caps?.emulate_uid.reason || '状态未知，等待后端返回'}
            </p>
          </Alert>

          {emulError && <Alert tone="danger">{emulError}</Alert>}
          {emulWarn && <Alert tone="warning">{emulWarn}</Alert>}
          {emul?.last_error && <Alert tone="danger">{emul.last_error}</Alert>}
          {emul?.note && <Alert tone="warning">{emul.note}</Alert>}

          <label className="flex flex-col gap-1">
            <span className="font-mono text-[10px] opacity-45">
              UID（4/7/10 字节 hex，可带冒号）
            </span>
            <input
              id="nfc-emul-uid"
              className="dm-input font-mono text-xs"
              placeholder="例如：db4c1b5a 或 04:A1:B2:C3:D4:E5:F6"
              value={emulUid}
              onChange={(e) => setEmulUid(e.target.value)}
              disabled={emulRunning || busy !== null}
            />
          </label>

          <div className="flex gap-2">
            {(['text', 'uri', 'message'] as const).map((kind) => (
              <Button
                key={kind}
                size="sm"
                variant={emulKind === kind ? 'secondary' : 'ghost'}
                isDisabled={emulRunning || busy !== null}
                onPress={() => setEmulKind(kind)}
                className="flex-1 font-mono text-xs"
              >
                {kind === 'text' ? '文本' : kind === 'uri' ? 'URI' : '克隆 NDEF'}
              </Button>
            ))}
          </div>

          {emulKind === 'message' ? (
            <label className="flex flex-col gap-1">
              <span className="font-mono text-[10px] opacity-45">
                NDEF 消息 hex（整条逐字节克隆；用读取结果的「用此内容模拟」自动带入）
              </span>
              <textarea
                className="dm-input max-h-28 min-h-16 font-mono text-[10px]"
                placeholder="d1010c5402656e48656c6c6f204e4643"
                value={emulRaw}
                onChange={(e) => setEmulRaw(e.target.value)}
                disabled={emulRunning || busy !== null}
              />
            </label>
          ) : (
            <input
              className="dm-input font-mono text-xs"
              placeholder={emulKind === 'text' ? '读卡器读到的文本内容' : 'https://example.com'}
              value={emulValue}
              onChange={(e) => setEmulValue(e.target.value)}
              disabled={emulRunning || busy !== null}
            />
          )}

          {emulKind === 'text' && (
            <label className="flex flex-col gap-1">
              <span className="font-mono text-[10px] opacity-45">语言码</span>
              <input
                className="dm-input w-24 py-1 font-mono text-xs"
                value={emulLang}
                onChange={(e) => setEmulLang(e.target.value)}
                disabled={emulRunning || busy !== null}
              />
            </label>
          )}

          <div className="flex gap-2">
            <Button
              size="sm"
              variant="secondary"
              isDisabled={emulRunning || busy !== null}
              onPress={() => handleEmulate('start')}
              className="flex-1 font-mono text-xs"
            >
              <RiPlayLine className="size-3.5" aria-hidden />
              {busy === 'emul-start' ? '启动中...' : '开始模拟'}
            </Button>
            <Button
              size="sm"
              variant={emulRunning ? 'danger' : 'ghost'}
              isDisabled={!emulRunning || busy !== null}
              onPress={() => handleEmulate('stop')}
              className="flex-1 font-mono text-xs"
            >
              <RiStopLine className="size-3.5" aria-hidden />
              {busy === 'emul-stop' ? '停止中...' : '停止模拟'}
            </Button>
          </div>

          {emul && (
            <div className="dm-inset px-2.5 py-1">
              <Row label="状态">
                {emul.running ? (emul.active ? '读卡器已激活' : '模拟中 · 等待读卡器') : '未模拟'}
              </Row>
              <Row label="模拟 UID">{emul.uid || '—'}</Row>
              <Row label="NDEF">{emul.ndef_hex || '—'}</Row>
              <Row label="开始于">{emul.started_at ?? '—'}</Row>
            </div>
          )}

          {emul?.events && emul.events.length > 0 && (
            <div className="dm-inset max-h-40 overflow-y-auto">
              {emul.events.map((ev, i) => (
                <div key={`${ev.at}-${i}`} className="dm-row flex items-start gap-2 px-2.5 py-1">
                  <span className="shrink-0 font-mono text-[10px] tabular-nums opacity-40">
                    {fmtTime(ev.at)}
                  </span>
                  <Chip size="sm" variant="secondary" color="default" className="shrink-0">
                    {ev.kind}
                  </Chip>
                  <span className="min-w-0 flex-1 break-all font-mono text-[10px]">{ev.text}</span>
                </div>
              ))}
            </div>
          )}

          <Note>
            模拟期间内核 NFC 临时下线（读写/轮询不可用），停止后自动交还驱动并恢复上电； 手机（NFC
            开启）贴近设备即可读到该 UID 与内容。克隆边界：MIFARE Classic 卡只能克隆 UID（CRYPTO1
            未实现），手机钱包不会把模拟卡识别为门禁卡；ATQA/SAK 与协议族由 ISO-DEP
            监听决定，非逐位复刻。
          </Note>
        </Panel>
      </Band>

      <Band index={4} title="事件日志" cols={1}>
        <Panel
          label="事件日志"
          index={8}
          hint={
            <>
              <span>最近 {visibleEvents.length} 条</span>
              <Button
                size="sm"
                variant="ghost"
                onPress={() => {
                  setEvents([]);
                  setClearedAt(Date.now());
                }}
                className="h-6 min-w-0 px-1.5 font-mono text-[10px]"
              >
                <RiDeleteBinLine className="size-3.5" aria-hidden />
                清屏
              </Button>
            </>
          }
        >
          {visibleEvents.length === 0 ? (
            <span className="font-mono text-[11px] opacity-35">
              暂无事件（接口不可用时保持空列表，不影响其它操作）
            </span>
          ) : (
            <div className="dm-inset max-h-72 overflow-y-auto">
              {visibleEvents.map((ev, i) => (
                <div
                  key={`${ev.at}-${ev.kind}-${i}`}
                  className="dm-row flex items-start gap-2 px-2.5 py-1.5"
                >
                  <span className="shrink-0 font-mono text-[10px] tabular-nums opacity-40">
                    {fmtTime(ev.at)}
                  </span>
                  <Chip size="sm" variant="secondary" color="default" className="shrink-0">
                    {ev.kind}
                  </Chip>
                  <span className="min-w-0 flex-1 break-all font-mono text-[10px]">{ev.text}</span>
                </div>
              ))}
            </div>
          )}
          <Note>每 2 秒拉取最近 50 条；清屏只在当前页面隐藏已显示内容，不修改后端记录。</Note>
        </Panel>
      </Band>

      <ConfirmModal
        open={confirmOpen}
        title="确认写入卡片"
        confirmLabel="确认写入"
        busy={busy === 'write'}
        onClose={() => setConfirmOpen(false)}
        onConfirm={handleWrite}
      >
        <div className="flex flex-col gap-2 text-[11px]">
          <Alert tone="warning">
            将覆盖卡片 NDEF 区：{selectedTag ? formatUid(selectedTag.uid) : '—'}（
            {protocolLabel(targetProtocol)}）
          </Alert>
          <div className="dm-inset px-2.5 py-1">
            <Row label="类型">{WRITE_KINDS.find(([k]) => k === writeKind)?.[1] ?? writeKind}</Row>
            <Row label="内容">
              <span className="break-all">
                {writeValue.length > 120 ? `${writeValue.slice(0, 120)}…` : writeValue || '—'}
              </span>
            </Row>
          </div>
        </div>
      </ConfirmModal>
    </div>
  );
}
