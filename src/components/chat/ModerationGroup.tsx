// The Moderation card: Announce, Unban requests (while someone asks), Stop
// running nuke (while one watches), Unpin message, Blocked terms and Clear
// chat, which asks in place. Shared by the Tools menu and the Manage channel
// window's Moderation page. Every result shows on its row; nothing toasts.
import { useEffect, useState } from 'react';
import { Ban, DoorOpen, Eraser, Megaphone, PinOff, Zap } from 'lucide-react';
import { invoke } from '@tauri-apps/api/core';
import { usePinStore } from '../../stores/pinStore';
import { readTool, runTool, toolKey, useManagerStore } from './managerStore';
import { GroupCard, MenuRow, RowButton, type Layout } from './managerKit';

const words = (e: unknown, fallback: string) => (typeof e === 'string' && e ? e : fallback);

export function ModerationGroup({
  layout,
  channelId,
  channelLogin,
  unbanCount,
  onOpen,
  onAnnounce,
  hideTitle = false,
}: {
  /** The window's Moderation page already says what this is. */
  hideTitle?: boolean;
  layout: Layout | 'menu';
  channelId: string;
  channelLogin: string;
  unbanCount: number;
  /** Open Blocked terms or Unban requests in place of the card. */
  onOpen: (page: 'blocked' | 'unban') => void;
  /** Open the Announce panel (the menu); the window has its own page. */
  onAnnounce?: () => void;
}) {
  const v = useManagerStore();
  const [nukes, setNukes] = useState(0);
  const [confirmClear, setConfirmClear] = useState(false);

  // Read when the card shows: the engine holds this in a plain module Map
  // with no store behind it.
  useEffect(() => {
    let alive = true;
    void invoke<number>('nuke_active', { channel: channelLogin })
      .then((n) => alive && setNukes(n))
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, [channelLogin]);

  const nukeKey = toolKey(channelId, 'mod:nuke');
  const unpinKey = toolKey(channelId, 'mod:unpin');
  const clearKey = toolKey(channelId, 'mod:clear');
  const nuke = readTool(v, nukeKey);
  const unpin = readTool(v, unpinKey);
  const clear = readTool(v, clearKey);
  const lineOf = (e: { phase: string; line: string | null }) => (e.phase === 'done' || e.phase === 'failed' ? e.line : null);

  return (
    <GroupCard title="Moderation" layout={layout} hideTitle={hideTitle}>
      {onAnnounce && <MenuRow layout={layout} icon={<Megaphone size={14} />} label="Announce" onClick={onAnnounce} />}
      {unbanCount > 0 && (
        <MenuRow
          layout={layout}
          icon={<DoorOpen size={14} />}
          label="Unban requests"
          trailing={<span className="text-[12px] tabular-nums text-textSecondary">{unbanCount}</span>}
          onClick={() => onOpen('unban')}
        />
      )}
      {(nukes > 0 || nuke.phase !== 'idle') && (
        <MenuRow
          layout={layout}
          icon={<Zap size={14} className="text-warning" />}
          label="Stop running nuke"
          line={lineOf(nuke)}
          failed={nuke.phase === 'failed'}
          trailing={
            nukes > 0 && (
              <RowButton
                layout={layout}
                disabled={nuke.phase === 'working'}
                onClick={() =>
                  void runTool(nukeKey, async () => {
                    try {
                      const n = await invoke<number>('nuke_stop', { channel: channelLogin });
                      setNukes(0);
                      return { ok: true, message: n === 1 ? 'Stopped 1 running nuke' : `Stopped ${n} running nukes` };
                    } catch (e) {
                      return { ok: false, message: words(e, 'Could not stop it') };
                    }
                  })
                }
              >
                Stop {nukes}
              </RowButton>
            )
          }
        />
      )}
      <MenuRow
        layout={layout}
        icon={<PinOff size={14} />}
        label="Unpin message"
        line={lineOf(unpin)}
        failed={unpin.phase === 'failed'}
        trailing={
          <RowButton
            layout={layout}
            disabled={unpin.phase === 'working'}
            onClick={() =>
              void runTool(unpinKey, async () => {
                try {
                  await invoke('unpin_chat_message', { broadcasterId: channelId });
                  usePinStore.getState().requestRefresh();
                  return { ok: true, message: 'Message unpinned' };
                } catch (e) {
                  return { ok: false, message: words(e, 'Nothing pinned right now') };
                }
              })
            }
          >
            Unpin
          </RowButton>
        }
      />
      <MenuRow layout={layout} icon={<Ban size={14} />} label="Blocked terms" onClick={() => onOpen('blocked')} />
      {confirmClear ? (
        <div
          onKeyDown={(e) => {
            if (e.key !== 'Escape') return;
            e.preventDefault();
            e.stopPropagation();
            setConfirmClear(false);
          }}
        >
          <MenuRow
            layout={layout}
            icon={<Eraser size={14} />}
            danger
            label="Clear everyone's chat?"
            below={
              <>
                <RowButton
                  layout={layout}
                  tone="commit"
                  autoFocus
                  onClick={() => {
                    setConfirmClear(false);
                    void runTool(clearKey, async () => {
                      try {
                        await invoke('clear_chat', { broadcasterId: channelId });
                        return { ok: true, message: 'Chat cleared' };
                      } catch (e) {
                        return { ok: false, message: words(e, 'Could not clear chat') };
                      }
                    });
                  }}
                >
                  Clear
                </RowButton>
                <RowButton layout={layout} onClick={() => setConfirmClear(false)}>
                  Cancel
                </RowButton>
              </>
            }
          />
        </div>
      ) : (
        <MenuRow
          layout={layout}
          icon={<Eraser size={14} />}
          label="Clear chat"
          line={lineOf(clear)}
          failed={clear.phase === 'failed'}
          trailing={
            <RowButton layout={layout} disabled={clear.phase === 'working'} onClick={() => setConfirmClear(true)}>
              Clear
            </RowButton>
          }
        />
      )}
    </GroupCard>
  );
}

export default ModerationGroup;
