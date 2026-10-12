// Editors tab: list a channel's 7TV editors, add new ones by Twitch name,
// adjust their permissions, and remove them. Only rendered when the signed-in
// user can manage editors for the channel (typically their own channel).
import { useCallback, useEffect, useState } from 'react';
import { Plus, Trash2, Check, X, Loader2, UserPlus } from 'lucide-react';
import { useAppStore } from '../../stores/AppStore';
import { Tooltip } from '../ui/Tooltip';
import {
  listEditors, addEditor, removeEditor, updateEditorPermissions, resolveTwitchUserTo7TV,
  SevenTVSessionExpired, type ChannelEditor, type EditableChannel,
} from '../../services/seventvEditorService';
import { SevenTvAvatar } from '../SevenTvAvatar';

interface Props {
  channel: EditableChannel;
}

export default function EditorsPanel({ channel }: Props) {
  const addToast = useAppStore((s) => s.addToast);
  const [editors, setEditors] = useState<ChannelEditor[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [adding, setAdding] = useState(false);
  const [lookup, setLookup] = useState('');
  const [busy, setBusy] = useState(false);

  const load = useCallback(async () => {
    setError(null);
    try {
      setEditors(await listEditors(channel.seventvUserId));
    } catch (e) {
      if (e instanceof SevenTVSessionExpired) addToast('Your 7TV session expired. Reconnect your 7TV account.', 'error');
      setError(e instanceof Error ? e.message : String(e));
      setEditors([]);
    }
  }, [channel.seventvUserId, addToast]);

  useEffect(() => {
    setEditors(null);
    load();
  }, [load]);

  const guard = async (fn: () => Promise<void>) => {
    setBusy(true);
    try {
      await fn();
      await load();
    } catch (e) {
      addToast(
        e instanceof SevenTVSessionExpired
          ? 'Your 7TV session expired. Reconnect your 7TV account.'
          : e instanceof Error ? e.message : 'Action failed',
        'error',
      );
    } finally {
      setBusy(false);
    }
  };

  const handleAdd = async () => {
    const raw = lookup.trim();
    if (!raw) return;
    setBusy(true);
    try {
      const found = await resolveTwitchUserTo7TV(raw);
      if (!found) {
        addToast(`Couldn't find a 7TV user for "${raw}"`, 'warning');
        return;
      }
      await addEditor(channel.seventvUserId, found.seventvUserId, { manage: true });
      addToast(`Invited ${found.displayName} as an editor`, 'success');
      setLookup('');
      setAdding(false);
      await load();
    } catch (e) {
      addToast(
        e instanceof SevenTVSessionExpired
          ? 'Your 7TV session expired. Reconnect your 7TV account.'
          : e instanceof Error ? e.message : 'Could not add editor',
        'error',
      );
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="h-full overflow-y-auto custom-scrollbar px-4 py-3">
      <div className="mb-3">
        {adding ? (
          <div className="flex items-center gap-2 max-w-md">
            <input
              autoFocus
              value={lookup}
              onChange={(e) => setLookup(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === 'Enter') handleAdd();
                else if (e.key === 'Escape') { setAdding(false); setLookup(''); }
              }}
              placeholder="Twitch username or id"
              className="flex-1 px-3 py-1.5 rounded-lg bg-glass text-sm text-textPrimary placeholder:text-textMuted outline-none focus:ring-1 focus:ring-accent/40"
            />
            <button
              disabled={!lookup.trim() || busy}
              onClick={handleAdd}
              className="glass-button px-3 py-1.5 rounded text-sm text-textPrimary disabled:opacity-50 flex items-center gap-1.5"
            >
              {busy ? <Loader2 size={14} className="animate-spin" /> : <UserPlus size={14} />}
              Invite
            </button>
            <button onClick={() => { setAdding(false); setLookup(''); }} className="p-1.5 text-textSecondary hover:text-textPrimary rounded">
              <X size={16} />
            </button>
          </div>
        ) : (
          <button onClick={() => setAdding(true)} className="glass-button px-3 py-1.5 rounded text-sm text-textPrimary flex items-center gap-1.5">
            <Plus size={15} /> Add editor
          </button>
        )}
        <p className="text-[11px] text-textMuted mt-1.5">
          Editors must accept the invite before they can make changes.
        </p>
      </div>

      {editors === null ? (
        <div className="flex items-center justify-center py-10">
          <Loader2 size={20} className="animate-spin text-textSecondary" />
        </div>
      ) : error ? (
        <p className="text-sm text-textSecondary text-center py-10 px-4">{error}</p>
      ) : editors.length === 0 ? (
        <p className="text-sm text-textMuted text-center py-10">No editors yet.</p>
      ) : (
        <div className="space-y-2">
          {editors.map((ed) => (
            <div key={ed.editorSeventvId} className="glass-panel rounded-lg p-3 flex items-center gap-3">
              {ed.avatarUrl ? (
                <SevenTvAvatar
                  cosmeticsId={ed.twitchId}
                  px={32}
                  src={ed.avatarUrl}
                  alt=""
                  className="w-8 h-8 rounded-full object-cover shrink-0"
                  loading="lazy"
                />
              ) : (
                <div className="w-8 h-8 rounded-full bg-glass flex items-center justify-center text-xs text-textSecondary shrink-0">
                  {ed.displayName.slice(0, 1).toUpperCase()}
                </div>
              )}
              <div className="min-w-0 flex-1">
                <div className="text-sm text-textPrimary truncate">{ed.displayName}</div>
                <div className="text-[11px] text-textMuted">
                  {ed.state === 'PENDING' ? 'invite pending' : ed.state === 'REJECTED' ? 'declined' : 'editor'}
                </div>
              </div>

              <div className="flex items-center gap-1 shrink-0">
                <PermToggle
                  label="Emotes"
                  on={ed.perms.manageEmotes}
                  disabled={busy}
                  onClick={() =>
                    guard(() =>
                      updateEditorPermissions(channel.seventvUserId, ed.editorSeventvId, {
                        manage: !ed.perms.manageEmotes,
                        admin: ed.perms.adminEmoteSets,
                        manageEditors: ed.perms.manageEditors,
                      }),
                    )
                  }
                />
                <PermToggle
                  label="Sets"
                  on={ed.perms.adminEmoteSets}
                  disabled={busy}
                  onClick={() =>
                    guard(() =>
                      updateEditorPermissions(channel.seventvUserId, ed.editorSeventvId, {
                        manage: ed.perms.manageEmotes,
                        admin: !ed.perms.adminEmoteSets,
                        manageEditors: ed.perms.manageEditors,
                      }),
                    )
                  }
                />
                <RemoveButton busy={busy} onConfirm={() => guard(() => removeEditor(channel.seventvUserId, ed.editorSeventvId))} />
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

function PermToggle({ label, on, disabled, onClick }: { label: string; on: boolean; disabled: boolean; onClick: () => void }) {
  return (
    <Tooltip content={on ? `Revoke: ${label}` : `Grant: ${label}`} side="top" delay={200}>
      <button
        onClick={onClick}
        disabled={disabled}
        className={`px-2 py-1 rounded-full text-[11px] border transition-colors disabled:opacity-50 ${
          on ? 'bg-glass-active text-textPrimary border-accent/40' : 'text-textSecondary border-borderSubtle hover:text-textPrimary'
        }`}
      >
        {label}
      </button>
    </Tooltip>
  );
}

function RemoveButton({ busy, onConfirm }: { busy: boolean; onConfirm: () => void }) {
  const [confirm, setConfirm] = useState(false);
  if (confirm) {
    return (
      <div className="flex items-center gap-1">
        <Tooltip content="Confirm remove editor" side="top" delay={150}>
          <button onClick={onConfirm} disabled={busy} className="p-1.5 rounded text-red-400 hover:bg-glass">
            <Check size={14} />
          </button>
        </Tooltip>
        <button onClick={() => setConfirm(false)} className="p-1.5 rounded text-textSecondary hover:bg-glass">
          <X size={14} />
        </button>
      </div>
    );
  }
  return (
    <Tooltip content="Remove editor" side="top" delay={200}>
      <button onClick={() => setConfirm(true)} className="p-1.5 rounded text-textSecondary hover:text-red-400 hover:bg-glass">
        <Trash2 size={14} />
      </button>
    </Tooltip>
  );
}
