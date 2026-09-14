import { useState, type CSSProperties } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useAppStore } from '../stores/AppStore';

/**
 * Mobile Twitch login. Desktop opens a WebView popup for the device-code
 * verification; mobile has no secondary window, so we show the code + verify URL
 * here and let the user authorize in their browser. The backend keeps polling
 * and emits `twitch-login-complete`, which clears `deviceCodeInfo`.
 */
export default function DeviceLoginOverlay() {
  const info = useAppStore((s) => s.deviceCodeInfo);
  const [copied, setCopied] = useState(false);
  if (!info) return null;

  const openBrowser = async () => {
    try {
      await invoke('open_browser_url', { url: info.verificationUri });
    } catch {
      /* No external browser (e.g. bare emulator) — the code works from any browser. */
    }
  };
  const copyCode = async () => {
    try {
      await navigator.clipboard.writeText(info.userCode);
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch {
      /* clipboard may be unavailable; the code is shown on screen regardless */
    }
  };
  const cancel = () => useAppStore.setState({ deviceCodeInfo: null, isLoading: false });

  const panel: CSSProperties = {
    width: '88%',
    maxWidth: 420,
    padding: '30px 24px',
    borderRadius: 20,
    background: 'rgba(24,24,27,0.94)',
    boxShadow: 'inset 0 0 0 1px rgba(255,255,255,0.07)',
    textAlign: 'center',
  };
  const codeBox: CSSProperties = {
    fontFamily: 'ui-monospace, SFMono-Regular, Menlo, monospace',
    fontSize: 34,
    letterSpacing: 6,
    fontWeight: 700,
    color: '#fff',
    padding: '14px 0',
    margin: '6px 0 4px',
    borderRadius: 12,
    background: 'rgba(255,255,255,0.04)',
    boxShadow: 'inset 0 0 0 1px rgba(255,255,255,0.06)',
    cursor: 'pointer',
  };
  const primaryBtn: CSSProperties = {
    width: '100%',
    padding: '13px 0',
    marginTop: 18,
    borderRadius: 12,
    border: 'none',
    background: '#9147ff',
    color: '#fff',
    fontSize: 15,
    fontWeight: 600,
    cursor: 'pointer',
  };

  return (
    <div
      style={{
        position: 'fixed',
        inset: 0,
        zIndex: 10000,
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
        background: 'rgba(0,0,0,0.72)',
        backdropFilter: 'blur(8px)',
        WebkitBackdropFilter: 'blur(8px)',
      }}
    >
      <div style={panel}>
        <div style={{ fontSize: 19, fontWeight: 700, color: '#fff', marginBottom: 6 }}>
          Log in to Twitch
        </div>
        <div style={{ fontSize: 13.5, color: 'rgba(255,255,255,0.55)', lineHeight: 1.5, marginBottom: 16 }}>
          Open the link below and enter this code to authorize StreamNook.
        </div>

        <div style={codeBox} onClick={copyCode} title="Tap to copy">
          {info.userCode}
        </div>
        <div style={{ fontSize: 12, color: 'rgba(255,255,255,0.4)', minHeight: 16 }}>
          {copied ? 'Copied' : 'Tap the code to copy'}
        </div>

        <button style={primaryBtn} onClick={openBrowser}>
          Open {info.verificationUri.replace(/^https?:\/\//, '')}
        </button>

        <div style={{ fontSize: 12.5, color: 'rgba(255,255,255,0.45)', marginTop: 14, display: 'flex', alignItems: 'center', justifyContent: 'center', gap: 8 }}>
          <span
            style={{
              width: 8,
              height: 8,
              borderRadius: 999,
              background: '#9147ff',
              display: 'inline-block',
              animation: 'sn-pulse 1.4s ease-in-out infinite',
            }}
          />
          Waiting for you to authorize…
        </div>

        <button
          onClick={cancel}
          style={{ marginTop: 16, background: 'none', border: 'none', color: 'rgba(255,255,255,0.4)', fontSize: 13, cursor: 'pointer' }}
        >
          Cancel
        </button>
      </div>
      <style>{`@keyframes sn-pulse{0%,100%{opacity:.35}50%{opacity:1}}`}</style>
    </div>
  );
}
