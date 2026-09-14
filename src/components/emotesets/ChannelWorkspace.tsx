// The right-hand workspace for one selected channel: a slim segment row
// (Emotes / Sets / Editors), the working-set selector + capacity, the
// "Add emotes" entry point, and the add-emotes drawer. Adding is the most
// frequent task, so the drawer is always one click from the Emotes tab.
import { useCallback, useEffect, useMemo, useState, type ReactNode } from 'react';
import { Plus, Layers, Users, Loader2, ChevronDown } from 'lucide-react';
import { SevenTVLogo } from './SevenTVLogo';
import { useAppStore } from '../../stores/AppStore';
import {
  getChannelSets,
  SevenTVSessionExpired,
  type ChannelSet,
  type EditableChannel,
} from '../../services/seventvEditorService';
import EmoteGrid from './EmoteGrid';
import AddEmotesDrawer from './AddEmotesDrawer';
import SetsPanel from './SetsPanel';
import EditorsPanel from './EditorsPanel';
import EmoteDetail, { type DetailContext } from './EmoteDetail';

export type WorkspaceTab = 'emotes' | 'sets' | 'editors';

interface Props {
  channel: EditableChannel;
  initialTab: WorkspaceTab;
  onChannelsChanged: () => void;
}

type SetsFetch = { list: ChannelSet[] } | { error: unknown };

export default function ChannelWorkspace({ channel, initialTab, onChannelsChanged }: Props) {
  const addToast = useAppStore((s) => s.addToast);

  const canManageEmotes = channel.perms.manageEmotes;
  const canManageEditors = channel.perms.manageEditors;

  const [tab, setTab] = useState<WorkspaceTab>(
    initialTab === 'editors' && !canManageEditors ? 'emotes' : initialTab,
  );
  const [sets, setSets] = useState<ChannelSet[] | null>(null);
  const [workingSetId, setWorkingSetId] = useState<string | null>(null);
  const [setsError, setSetsError] = useState<string | null>(null);
  const [addOpen, setAddOpen] = useState(false);
  const [reloadKey, setReloadKey] = useState(0);
  const [emoteCount, setEmoteCount] = useState<number | null>(null);
  const [setMenuOpen, setSetMenuOpen] = useState(false);
  const [detail, setDetail] = useState<DetailContext | null>(null);

  // Fetch and apply are separate so state is only ever written from a
  // promise continuation, never synchronously inside an effect body.
  const fetchSets = useCallback(async (): Promise<SetsFetch> => {
    try {
      // Personal set is editable only on your own channel.
      const list = await getChannelSets(channel.seventvUserId, undefined, channel.isSelf);
      return { list };
    } catch (error) {
      return { error };
    }
  }, [channel.seventvUserId, channel.isSelf]);

  const applySets = useCallback((result: SetsFetch) => {
    if ('list' in result) {
      const list = result.list;
      setSets(list);
      setWorkingSetId((prev) => {
        if (prev && list.some((s) => s.id === prev)) return prev;
        const active = list.find((s) => s.isActive);
        return active?.id ?? list[0]?.id ?? null;
      });
      return;
    }
    const e = result.error;
    if (e instanceof SevenTVSessionExpired) {
      addToast('Your 7TV session expired. Reconnect your 7TV account.', 'error');
    }
    setSetsError(e instanceof Error ? e.message : String(e));
    setSets([]);
  }, [addToast]);

  // A different channel is a fresh workspace: clear the set list during
  // render (adjust-state-on-prop-change) so the previous channel's sets never
  // paint while the new ones load.
  const channelKey = `${channel.seventvUserId}:${channel.isSelf ? 1 : 0}`;
  const [loadedChannelKey, setLoadedChannelKey] = useState(channelKey);
  if (channelKey !== loadedChannelKey) {
    setLoadedChannelKey(channelKey);
    setSets(null);
    setWorkingSetId(null);
    setEmoteCount(null);
    setSetsError(null);
  }

  useEffect(() => {
    // A fetch that resolves after the channel changed (or the workspace
    // unmounted) is dropped rather than applied to the wrong channel.
    let alive = true;
    void fetchSets().then((result) => {
      if (alive) applySets(result);
    });
    return () => {
      alive = false;
    };
  }, [fetchSets, applySets]);

  const workingSet = useMemo(
    () => sets?.find((s) => s.id === workingSetId) || null,
    [sets, workingSetId],
  );

  const onSetsMutated = useCallback(() => {
    setSetsError(null);
    void fetchSets().then(applySets);
    onChannelsChanged();
  }, [fetchSets, applySets, onChannelsChanged]);

  const tabs: { id: WorkspaceTab; label: string; icon: ReactNode; show: boolean }[] = [
    { id: 'emotes', label: 'Emotes', icon: <SevenTVLogo className="h-3.5 w-auto" />, show: true },
    { id: 'sets', label: 'Sets', icon: <Layers size={14} />, show: true },
    { id: 'editors', label: 'Editors', icon: <Users size={14} />, show: canManageEditors },
  ];

  return (
    <div className="h-full flex flex-col min-w-0">
      {/* Workspace header */}
      <div className="px-4 pt-3 pb-2 border-b border-borderSubtle shrink-0">
        <div className="flex items-center justify-between gap-3">
          <div className="min-w-0">
            <div className="text-base font-semibold text-textPrimary truncate">{channel.displayName}</div>
            {!canManageEmotes && (
              <div className="text-[11px] text-amber-400/90">
                View only. You don't have permission to change this channel's emotes.
              </div>
            )}
          </div>
          {/* Segment tabs */}
          <div className="flex items-center gap-1 bg-glass rounded-lg p-0.5 shrink-0">
            {tabs.filter((t) => t.show).map((t) => {
              const active = tab === t.id;
              return (
                <button
                  key={t.id}
                  onClick={() => setTab(t.id)}
                  className={`flex items-center gap-1.5 px-3 py-1.5 rounded-md text-sm transition-colors ${
                    active ? 'bg-glass-active text-textPrimary' : 'text-textSecondary hover:text-textPrimary'
                  }`}
                >
                  {t.icon}
                  {t.label}
                </button>
              );
            })}
          </div>
        </div>

        {/* Working-set selector + capacity + add (Emotes tab only) */}
        {tab === 'emotes' && (
          <div className="flex items-center justify-between gap-3 mt-3">
            <div className="flex items-center gap-3 min-w-0">
              <div className="relative">
                <button
                  onClick={() => setSetMenuOpen((v) => !v)}
                  disabled={!sets || sets.length === 0}
                  className="flex items-center gap-1.5 glass-button px-3 py-1.5 rounded text-sm text-textPrimary disabled:opacity-50 max-w-[260px]"
                >
                  <span className="truncate">{workingSet ? workingSet.name : 'No set'}</span>
                  {workingSet?.isActive && (
                    <span className="text-[10px] text-emerald-400 shrink-0">active</span>
                  )}
                  <ChevronDown size={14} className="shrink-0 text-textSecondary" />
                </button>
                {setMenuOpen && sets && (
                  <>
                    <div className="fixed inset-0 z-10" onClick={() => setSetMenuOpen(false)} />
                    <div className="absolute left-0 top-full mt-1 z-20 min-w-[220px] rounded-lg border border-borderSubtle bg-tertiary shadow-[0_12px_32px_rgba(0,0,0,0.5)] p-1 max-h-72 overflow-y-auto custom-scrollbar">
                      {sets.map((s) => (
                        <button
                          key={s.id}
                          onClick={() => {
                            setWorkingSetId(s.id);
                            setSetMenuOpen(false);
                          }}
                          className={`w-full text-left px-2 py-1.5 rounded text-sm flex items-center justify-between gap-2 ${
                            s.id === workingSetId ? 'bg-glass-active text-textPrimary' : 'text-textSecondary hover:bg-glass'
                          }`}
                        >
                          <span className="truncate">{s.name}</span>
                          <span className="flex items-center gap-1.5 shrink-0">
                            {s.kind === 'PERSONAL' && <span className="text-[10px] text-textMuted">personal</span>}
                            {s.isActive && <span className="text-[10px] text-emerald-400">active</span>}
                          </span>
                        </button>
                      ))}
                    </div>
                  </>
                )}
              </div>
              {workingSet && (
                <CapacityMeter used={emoteCount} capacity={workingSet.capacity} />
              )}
            </div>
            {canManageEmotes && workingSet && (
              <button
                onClick={() => setAddOpen(true)}
                className="glass-button px-3 py-1.5 rounded text-sm text-textPrimary flex items-center gap-1.5 shrink-0"
              >
                <Plus size={15} />
                Add emotes
              </button>
            )}
          </div>
        )}
      </div>

      {/* Tab content */}
      <div className="flex-1 min-h-0 relative">
        {tab === 'emotes' && (
          sets === null ? (
            <Centered>
              <Loader2 size={20} className="animate-spin text-textSecondary" />
            </Centered>
          ) : setsError ? (
            <Centered>
              <p className="text-sm text-textSecondary max-w-md text-center px-6">{setsError}</p>
            </Centered>
          ) : !workingSet ? (
            <Centered>
              <p className="text-sm text-textMuted text-center px-6">
                This channel has no emote set yet.
                {canManageEmotes ? ' Create one from the Sets tab.' : ''}
              </p>
            </Centered>
          ) : (
            <EmoteGrid
              setId={workingSet.id}
              canManage={canManageEmotes}
              reloadKey={reloadKey}
              onCountChange={setEmoteCount}
              onOpenDetail={(e) =>
                setDetail({
                  emoteId: e.emoteId,
                  defaultName: e.defaultName,
                  inSet: { setId: workingSet.id, alias: e.alias, zeroWidth: e.zeroWidth },
                })
              }
            />
          )
        )}

        {tab === 'sets' && (
          <SetsPanel
            channel={channel}
            sets={sets}
            onMutated={onSetsMutated}
            onOpenSet={(id) => {
              setWorkingSetId(id);
              setTab('emotes');
            }}
          />
        )}

        {tab === 'editors' && canManageEditors && (
          <EditorsPanel channel={channel} />
        )}

        {/* Add-emotes drawer */}
        {addOpen && workingSet && (
          <AddEmotesDrawer
            setId={workingSet.id}
            setName={workingSet.name}
            onClose={() => setAddOpen(false)}
            onAdded={() => {
              setReloadKey((k) => k + 1);
            }}
            onOpenDetail={(e) => setDetail({ emoteId: e.id, defaultName: e.defaultName })}
          />
        )}

        {/* Emote detail (over everything, from grid or search) */}
        {detail && (
          <EmoteDetail
            ctx={detail}
            canManage={canManageEmotes}
            sets={sets}
            workingSetId={workingSetId}
            onClose={() => setDetail(null)}
            onChanged={() => setReloadKey((k) => k + 1)}
          />
        )}
      </div>
    </div>
  );
}

function CapacityMeter({ used, capacity }: { used: number | null; capacity?: number }) {
  if (used == null && capacity == null) return null;
  const pct = capacity && used != null ? Math.min(100, Math.round((used / capacity) * 100)) : 0;
  const full = capacity != null && used != null && used >= capacity;
  return (
    <div className="flex items-center gap-2 text-xs text-textSecondary">
      <span className={full ? 'text-amber-400' : ''}>
        {used ?? '—'}
        {capacity != null ? ` / ${capacity}` : ''}
      </span>
      {capacity != null && (
        <div className="w-24 h-1.5 rounded-full bg-glass overflow-hidden">
          <div
            className={`h-full rounded-full ${full ? 'bg-amber-400' : 'bg-accent'}`}
            style={{ width: `${pct}%` }}
          />
        </div>
      )}
    </div>
  );
}

function Centered({ children }: { children: ReactNode }) {
  return <div className="absolute inset-0 flex items-center justify-center">{children}</div>;
}
