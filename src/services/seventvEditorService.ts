// 7TV editor service: read + write operations for managing emote sets and
// editors on channels the signed-in 7TV account can edit. Everything routes
// through the authenticated passthrough command (`seventv_graphql_authed`),
// because the whole feature is gated behind a connected 7TV account anyway and
// the authed path is the only one that supports GraphQL variables. The exact
// v4 schema shapes used here were confirmed by live introspection of
// https://7tv.io/v4/gql rather than taken from documentation.
import { invoke } from '@tauri-apps/api/core';
import { Logger } from '../utils/logger';

// ── Errors ──────────────────────────────────────────────────────────────────

/** The stored 7TV token is gone or rejected. UI should prompt a reconnect. */
export class SevenTVSessionExpired extends Error {
  constructor(message = 'Your 7TV session has expired. Reconnect your 7TV account.') {
    super(message);
    this.name = 'SevenTVSessionExpired';
  }
}

/** A non-auth GraphQL error (capacity full, name conflict, permission, etc). */
export class SevenTVGraphQLError extends Error {
  errors: any[];
  code?: string;
  constructor(errors: any[]) {
    const first = Array.isArray(errors) ? errors[0] : undefined;
    super(first?.message || 'A 7TV request failed.');
    this.name = 'SevenTVGraphQLError';
    this.errors = errors || [];
    this.code = first?.extensions?.code;
  }
}

// ── Types ─────────────────────────────────────────────────────────────────

export type EditorState = 'PENDING' | 'ACCEPTED' | 'REJECTED';
export type EmoteSetKind = 'NORMAL' | 'PERSONAL' | 'GLOBAL' | 'SPECIAL';

export type SortBy =
  | 'TRENDING_DAILY' | 'TRENDING_WEEKLY' | 'TRENDING_MONTHLY'
  | 'TOP_DAILY' | 'TOP_WEEKLY' | 'TOP_MONTHLY' | 'TOP_ALL_TIME'
  | 'NAME_ALPHABETICAL' | 'UPLOAD_DATE';
export type SortOrder = 'ASCENDING' | 'DESCENDING';

export interface DirectoryFilters {
  animated?: boolean;
  defaultZeroWidth?: boolean;
  nsfw?: boolean;
  exactMatch?: boolean;
}

/** Permission flags relevant to this dashboard (a lean read of the full set). */
export interface ChannelPerms {
  /** Can add/remove/rename emotes in this channel's sets. */
  manageEmotes: boolean;
  /** Can rename/resize/delete sets and switch the active set. */
  adminEmoteSets: boolean;
  /** Can add/remove editors on this channel. */
  manageEditors: boolean;
}

export interface EditableChannel {
  seventvUserId: string;
  twitchId?: string;
  username: string;
  displayName: string;
  avatarUrl?: string;
  isSelf: boolean;
  inviteState: EditorState;
  perms: ChannelPerms;
  activeEmoteSetId?: string;
}

export interface ChannelSet {
  id: string;
  name: string;
  capacity?: number;
  kind: EmoteSetKind;
  isActive: boolean;
}

export interface SetEmote {
  /** The underlying emote id (used for CDN URLs and mutation ids). */
  emoteId: string;
  /** The display alias inside this set. */
  alias: string;
  /** The emote's original (default) name. */
  defaultName: string;
  zeroWidth: boolean;
  animated: boolean;
  addedAt?: string;
  addedById?: string;
}

export interface SetEmotesPage {
  setId: string;
  name: string;
  capacity?: number;
  emotes: SetEmote[];
  totalCount: number;
  pageCount: number;
}

export interface DirectoryEmote {
  id: string;
  defaultName: string;
  animated: boolean;
  zeroWidth: boolean;
  nsfw: boolean;
  ownerName?: string;
}

export interface DirectoryPage {
  emotes: DirectoryEmote[];
  totalCount: number;
  pageCount: number;
}

export interface ChannelEditor {
  editorSeventvId: string;
  username: string;
  displayName: string;
  avatarUrl?: string;
  state: EditorState;
  perms: ChannelPerms;
}

// ── GraphQL transport ───────────────────────────────────────────────────────

interface AuthedResponse {
  data?: any;
  errors?: any[];
  message?: string;
}

const cleanQuery = (q: string): string => q.replace(/\s+/g, ' ').trim();

/**
 * Run an authenticated 7TV GraphQL operation (read or mutation) and return the
 * `data` payload. Throws SevenTVSessionExpired when the token is dead and
 * SevenTVGraphQLError for any other GraphQL error so callers can show a precise
 * message (capacity full, name conflict, missing permission).
 */
async function gqlAuthed(
  query: string,
  variables?: Record<string, any>,
  operationName?: string,
  accountId?: string,
): Promise<any> {
  let res: AuthedResponse;
  try {
    res = (await invoke('seventv_graphql_authed', {
      query: cleanQuery(query),
      variables: variables ?? null,
      operationName: operationName ?? null,
      accountId: accountId ?? null,
    })) as AuthedResponse;
  } catch (e) {
    const msg = String(e);
    if (msg.includes('SESSION_EXPIRED') || msg.includes('Not authenticated')) {
      throw new SevenTVSessionExpired();
    }
    Logger.error('[7TV editor] request failed:', msg);
    throw new Error(msg);
  }
  if (res?.errors && res.errors.length) {
    const err = new SevenTVGraphQLError(res.errors);
    if (err.code === 'LOGIN_REQUIRED') throw new SevenTVSessionExpired();
    throw err;
  }
  return res?.data;
}

// ── Shared field selections ───────────────────────────────────────────────

const USER_IDENTITY = /* GraphQL */ `
  id
  mainConnection { platform platformId platformUsername platformDisplayName platformAvatarUrl }
  style { activeEmoteSetId }
`;

const LEAN_PERMS = /* GraphQL */ `
  emoteSet { admin manage }
  user { manageEditors }
`;

// ── Parsing helpers ───────────────────────────────────────────────────────

const parsePerms = (p: any): ChannelPerms => ({
  manageEmotes: !!(p?.emoteSet?.manage || p?.emoteSet?.admin || p?.superAdmin),
  adminEmoteSets: !!(p?.emoteSet?.admin || p?.superAdmin),
  manageEditors: !!(p?.user?.manageEditors || p?.superAdmin),
});

const twitchIdOf = (u: any): string | undefined =>
  u?.mainConnection?.platform === 'TWITCH' ? u.mainConnection.platformId : undefined;

const channelFromUser = (
  u: any,
  opts: { isSelf: boolean; state: EditorState; perms: ChannelPerms },
): EditableChannel => ({
  seventvUserId: u.id,
  twitchId: twitchIdOf(u),
  username: u?.mainConnection?.platformUsername || u.id,
  displayName: u?.mainConnection?.platformDisplayName || u?.mainConnection?.platformUsername || u.id,
  avatarUrl: u?.mainConnection?.platformAvatarUrl || undefined,
  isSelf: opts.isSelf,
  inviteState: opts.state,
  perms: opts.perms,
  activeEmoteSetId: u?.style?.activeEmoteSetId || undefined,
});

// ── Reads ───────────────────────────────────────────────────────────────────

let editableChannelsCache: { at: number; accountId: string; value: EditableChannel[] } | null = null;
const EDITABLE_TTL = 60 * 1000;

/**
 * The channels the signed-in 7TV user can act on: their own channel first, then
 * every channel they're an editor of (including PENDING invites so the UI can
 * offer accept/decline). `force` bypasses the short cache (e.g. after accepting
 * an invite).
 */
export async function getEditableChannels(
  accountId?: string,
  force = false,
): Promise<EditableChannel[]> {
  const key = accountId || '';
  const now = Date.now();
  if (!force && editableChannelsCache && editableChannelsCache.accountId === key && now - editableChannelsCache.at < EDITABLE_TTL) {
    return editableChannelsCache.value;
  }

  const query = /* GraphQL */ `
    query EditableChannels {
      users {
        me {
          ${USER_IDENTITY}
          editorFor {
            state
            permissions { ${LEAN_PERMS} }
            user { ${USER_IDENTITY} }
          }
        }
      }
    }
  `;
  const data = await gqlAuthed(query, undefined, 'EditableChannels', accountId);
  const me = data?.users?.me;
  if (!me) return [];

  const channels: EditableChannel[] = [];
  // You implicitly have full rights on your own channel.
  channels.push(
    channelFromUser(me, {
      isSelf: true,
      state: 'ACCEPTED',
      perms: { manageEmotes: true, adminEmoteSets: true, manageEditors: true },
    }),
  );
  for (const ed of me.editorFor ?? []) {
    if (!ed?.user) continue;
    channels.push(
      channelFromUser(ed.user, {
        isSelf: false,
        state: (ed.state as EditorState) || 'ACCEPTED',
        perms: parsePerms(ed.permissions),
      }),
    );
  }

  editableChannelsCache = { at: now, accountId: key, value: channels };
  return channels;
}

export function invalidateEditableChannels(): void {
  editableChannelsCache = null;
}

/**
 * All emote sets belonging to a channel, with the active one flagged. When
 * `includePersonal` is set, the user's personal emote set is appended too: it is
 * a SEPARATE field in the v4 schema (`user.personalEmoteSet`), not part of
 * `emoteSets`, and only the owner can edit it, so callers pass it only for the
 * signed-in user's own channel. Its emotes work in any channel, not just here.
 */
export async function getChannelSets(
  seventvUserId: string,
  accountId?: string,
  includePersonal = false,
): Promise<ChannelSet[]> {
  const query = /* GraphQL */ `
    query ChannelSets($id: Id!) {
      users { user(id: $id) {
        style { activeEmoteSetId }
        emoteSets { id name capacity kind }
        ${includePersonal ? 'personalEmoteSet { id name capacity }' : ''}
      } }
    }
  `;
  const data = await gqlAuthed(query, { id: seventvUserId }, 'ChannelSets', accountId);
  const u = data?.users?.user;
  if (!u) return [];
  const activeId = u.style?.activeEmoteSetId;
  const sets: ChannelSet[] = (u.emoteSets ?? []).map((s: any): ChannelSet => ({
    id: s.id,
    name: s.name,
    capacity: s.capacity ?? undefined,
    kind: (s.kind as EmoteSetKind) || 'NORMAL',
    isActive: s.id === activeId,
  }));
  const personal = includePersonal ? u.personalEmoteSet : null;
  if (personal?.id && !sets.some((s) => s.id === personal.id)) {
    sets.push({
      id: personal.id,
      name: personal.name || 'Personal Emotes',
      capacity: personal.capacity ?? undefined,
      kind: 'PERSONAL',
      isActive: false,
    });
  }
  return sets;
}

/** A page of a set's emotes, optionally filtered by an in-set name query. */
export async function getSetEmotes(
  setId: string,
  page = 1,
  perPage = 100,
  query?: string,
  accountId?: string,
): Promise<SetEmotesPage> {
  const gql = /* GraphQL */ `
    query SetEmotes($id: Id!, $page: Int!, $perPage: Int!, $query: String) {
      emoteSets { emoteSet(id: $id) {
        id name capacity
        emotes(query: $query, page: $page, perPage: $perPage) {
          items { alias addedAt addedById flags { zeroWidth } emote { id defaultName flags { animated } } }
          totalCount pageCount
        }
      } }
    }
  `;
  const data = await gqlAuthed(
    gql,
    { id: setId, page, perPage, query: query || null },
    'SetEmotes',
    accountId,
  );
  const set = data?.emoteSets?.emoteSet;
  const emotesNode = set?.emotes;
  const emotes: SetEmote[] = (emotesNode?.items ?? []).map((it: any): SetEmote => ({
    emoteId: it.emote?.id,
    alias: it.alias,
    defaultName: it.emote?.defaultName || it.alias,
    zeroWidth: !!it.flags?.zeroWidth,
    animated: !!it.emote?.flags?.animated,
    addedAt: it.addedAt,
    addedById: it.addedById ?? undefined,
  }));
  return {
    setId,
    name: set?.name || '',
    capacity: set?.capacity ?? undefined,
    emotes,
    totalCount: emotesNode?.totalCount ?? emotes.length,
    pageCount: emotesNode?.pageCount ?? 1,
  };
}

/** Search the 7TV emote directory (the "add emotes" flow). */
export async function searchDirectory(
  query: string,
  opts?: { sortBy?: SortBy; order?: SortOrder; filters?: DirectoryFilters; page?: number; perPage?: number },
  accountId?: string,
): Promise<DirectoryPage> {
  const sortBy: SortBy = opts?.sortBy || (query ? 'TOP_ALL_TIME' : 'TRENDING_WEEKLY');
  const order: SortOrder = opts?.order || 'DESCENDING';
  const gql = /* GraphQL */ `
    query SearchEmotes($query: String, $sort: Sort!, $filters: Filters, $page: Int, $perPage: Int) {
      emotes { search(query: $query, sort: $sort, filters: $filters, page: $page, perPage: $perPage) {
        items { id defaultName flags { animated defaultZeroWidth nsfw } owner { mainConnection { platformDisplayName platformUsername } } }
        totalCount pageCount
      } }
    }
  `;
  const filters = opts?.filters
    ? {
        animated: opts.filters.animated ?? null,
        defaultZeroWidth: opts.filters.defaultZeroWidth ?? null,
        nsfw: opts.filters.nsfw ?? null,
        exactMatch: opts.filters.exactMatch ?? null,
      }
    : null;
  const data = await gqlAuthed(
    gql,
    { query: query || null, sort: { sortBy, order }, filters, page: opts?.page ?? 1, perPage: opts?.perPage ?? 60 },
    'SearchEmotes',
    accountId,
  );
  const node = data?.emotes?.search;
  const emotes: DirectoryEmote[] = (node?.items ?? []).map((e: any): DirectoryEmote => ({
    id: e.id,
    defaultName: e.defaultName,
    animated: !!e.flags?.animated,
    zeroWidth: !!e.flags?.defaultZeroWidth,
    nsfw: !!e.flags?.nsfw,
    ownerName: e.owner?.mainConnection?.platformDisplayName || e.owner?.mainConnection?.platformUsername || undefined,
  }));
  return { emotes, totalCount: node?.totalCount ?? emotes.length, pageCount: node?.pageCount ?? 1 };
}

/** Resolve an emote id or a 7tv.app emote URL to a directory emote. */
export async function resolveEmote(input: string, accountId?: string): Promise<DirectoryEmote | null> {
  const trimmed = input.trim();
  // Accept raw ids and any 7tv.app/emotes/<id> URL form.
  const match = trimmed.match(/emotes\/([0-9A-Za-z]+)/) || trimmed.match(/^([0-9A-Za-z]{20,})$/);
  const id = match?.[1];
  if (!id) return null;
  const gql = /* GraphQL */ `
    query ResolveEmote($id: Id!) {
      emotes { emote(id: $id) { id defaultName flags { animated defaultZeroWidth nsfw } owner { mainConnection { platformDisplayName platformUsername } } } }
    }
  `;
  const data = await gqlAuthed(gql, { id }, 'ResolveEmote', accountId);
  const e = data?.emotes?.emote;
  if (!e) return null;
  return {
    id: e.id,
    defaultName: e.defaultName,
    animated: !!e.flags?.animated,
    zeroWidth: !!e.flags?.defaultZeroWidth,
    nsfw: !!e.flags?.nsfw,
    ownerName: e.owner?.mainConnection?.platformDisplayName || e.owner?.mainConnection?.platformUsername || undefined,
  };
}

export interface EmoteDetail {
  id: string;
  defaultName: string;
  tags: string[];
  animated: boolean;
  zeroWidth: boolean;
  nsfw: boolean;
  ownerId?: string;
  ownerName?: string;
  ownerAvatarUrl?: string;
  /** How many channels currently use this emote (popularity). */
  channelCount?: number;
}

/** Full info for one emote, for the detail view. */
export async function getEmoteDetail(emoteId: string, accountId?: string): Promise<EmoteDetail | null> {
  const gql = /* GraphQL */ `
    query EmoteDetail($id: Id!) {
      emotes { emote(id: $id) {
        id defaultName tags
        flags { animated defaultZeroWidth nsfw }
        owner { id mainConnection { platformUsername platformDisplayName platformAvatarUrl } }
        channels { totalCount }
      } }
    }
  `;
  const data = await gqlAuthed(gql, { id: emoteId }, 'EmoteDetail', accountId);
  const e = data?.emotes?.emote;
  if (!e) return null;
  return {
    id: e.id,
    defaultName: e.defaultName,
    tags: e.tags ?? [],
    animated: !!e.flags?.animated,
    zeroWidth: !!e.flags?.defaultZeroWidth,
    nsfw: !!e.flags?.nsfw,
    ownerId: e.owner?.id,
    ownerName: e.owner?.mainConnection?.platformDisplayName || e.owner?.mainConnection?.platformUsername || undefined,
    ownerAvatarUrl: e.owner?.mainConnection?.platformAvatarUrl || undefined,
    channelCount: e.channels?.totalCount,
  };
}

/** Editors of a channel, with their state and permissions. */
export async function listEditors(seventvUserId: string, accountId?: string): Promise<ChannelEditor[]> {
  const gql = /* GraphQL */ `
    query ListEditors($id: Id!) {
      users { user(id: $id) {
        editors {
          editorId state
          permissions { ${LEAN_PERMS} }
          editor { ${USER_IDENTITY} }
        }
      } }
    }
  `;
  const data = await gqlAuthed(gql, { id: seventvUserId }, 'ListEditors', accountId);
  const editors = data?.users?.user?.editors ?? [];
  return editors.map((e: any): ChannelEditor => ({
    editorSeventvId: e.editorId,
    username: e.editor?.mainConnection?.platformUsername || e.editorId,
    displayName: e.editor?.mainConnection?.platformDisplayName || e.editor?.mainConnection?.platformUsername || e.editorId,
    avatarUrl: e.editor?.mainConnection?.platformAvatarUrl || undefined,
    state: (e.state as EditorState) || 'ACCEPTED',
    perms: parsePerms(e.permissions),
  }));
}

/** Resolve a Twitch login (or numeric id) to a 7TV user id for editor invites. */
export async function resolveTwitchUserTo7TV(
  twitchLoginOrId: string,
  accountId?: string,
): Promise<{ seventvUserId: string; displayName: string } | null> {
  const raw = twitchLoginOrId.trim().replace(/^@/, '');
  // userByConnection needs the numeric Twitch id; a search covers login names.
  const gql = /* GraphQL */ `
    query FindUser($query: String!) {
      users { search(query: $query, page: 1, perPage: 5) {
        items { id mainConnection { platform platformId platformUsername platformDisplayName } }
      } }
    }
  `;
  const data = await gqlAuthed(gql, { query: raw }, 'FindUser', accountId);
  const items = data?.users?.search?.items ?? [];
  const hit = items.find(
    (u: any) =>
      u?.mainConnection?.platform === 'TWITCH' &&
      (u.mainConnection.platformUsername?.toLowerCase() === raw.toLowerCase() ||
        u.mainConnection.platformId === raw),
  ) || items[0];
  if (!hit) return null;
  return {
    seventvUserId: hit.id,
    displayName: hit.mainConnection?.platformDisplayName || hit.mainConnection?.platformUsername || hit.id,
  };
}

// ── Writes (mutations) ───────────────────────────────────────────────────────

interface EmoteRef {
  emoteId: string;
  alias?: string;
}

const emoteRef = (emoteId: string, alias?: string): EmoteRef =>
  alias ? { emoteId, alias } : { emoteId };

/** Add an emote to a set, optionally with a custom alias and zero-width. */
export async function addEmote(
  setId: string,
  emoteId: string,
  opts?: { alias?: string; zeroWidth?: boolean; overrideConflicts?: boolean },
  accountId?: string,
): Promise<void> {
  const gql = /* GraphQL */ `
    mutation AddEmote($setId: Id!, $emote: EmoteSetEmoteId!, $zeroWidth: Boolean, $override: Boolean) {
      emoteSets { emoteSet(id: $setId) { addEmote(id: $emote, zeroWidth: $zeroWidth, overrideConflicts: $override) { id } } }
    }
  `;
  await gqlAuthed(
    gql,
    { setId, emote: emoteRef(emoteId, opts?.alias), zeroWidth: opts?.zeroWidth ?? null, override: opts?.overrideConflicts ?? null },
    'AddEmote',
    accountId,
  );
}

/** Remove an emote from a set. Pass the alias to disambiguate duplicates. */
export async function removeEmote(setId: string, emoteId: string, alias?: string, accountId?: string): Promise<void> {
  const gql = /* GraphQL */ `
    mutation RemoveEmote($setId: Id!, $emote: EmoteSetEmoteId!) {
      emoteSets { emoteSet(id: $setId) { removeEmote(id: $emote) { id } } }
    }
  `;
  await gqlAuthed(gql, { setId, emote: emoteRef(emoteId, alias) }, 'RemoveEmote', accountId);
}

/** Rename (re-alias) an emote within a set. */
export async function renameEmote(
  setId: string,
  emoteId: string,
  newAlias: string,
  currentAlias?: string,
  accountId?: string,
): Promise<void> {
  const gql = /* GraphQL */ `
    mutation RenameEmote($setId: Id!, $emote: EmoteSetEmoteId!, $alias: String!) {
      emoteSets { emoteSet(id: $setId) { updateEmoteAlias(id: $emote, alias: $alias) { alias } } }
    }
  `;
  await gqlAuthed(gql, { setId, emote: emoteRef(emoteId, currentAlias), alias: newAlias }, 'RenameEmote', accountId);
}

/** Toggle the zero-width (overlay) flag on an emote in a set. */
export async function setEmoteZeroWidth(
  setId: string,
  emoteId: string,
  zeroWidth: boolean,
  alias?: string,
  accountId?: string,
): Promise<void> {
  const gql = /* GraphQL */ `
    mutation SetEmoteFlags($setId: Id!, $emote: EmoteSetEmoteId!, $flags: EmoteSetEmoteFlagsInput!) {
      emoteSets { emoteSet(id: $setId) { updateEmoteFlags(id: $emote, flags: $flags) { flags { zeroWidth } } } }
    }
  `;
  await gqlAuthed(
    gql,
    { setId, emote: emoteRef(emoteId, alias), flags: { zeroWidth, overrideConflicts: false } },
    'SetEmoteFlags',
    accountId,
  );
}

/** Create a new emote set for a channel (ownerId omitted = your own channel). */
export async function createSet(name: string, ownerId?: string, accountId?: string): Promise<string | undefined> {
  const gql = /* GraphQL */ `
    mutation CreateSet($name: String!, $tags: [String!]!, $ownerId: Id) {
      emoteSets { create(name: $name, tags: $tags, ownerId: $ownerId) { id } }
    }
  `;
  const data = await gqlAuthed(gql, { name, tags: [], ownerId: ownerId ?? null }, 'CreateSet', accountId);
  return data?.emoteSets?.create?.id;
}

export async function renameSet(setId: string, name: string, accountId?: string): Promise<void> {
  const gql = /* GraphQL */ `
    mutation RenameSet($setId: Id!, $name: String!) {
      emoteSets { emoteSet(id: $setId) { name(name: $name) { id } } }
    }
  `;
  await gqlAuthed(gql, { setId, name }, 'RenameSet', accountId);
}

export async function setSetCapacity(setId: string, capacity: number, accountId?: string): Promise<void> {
  const gql = /* GraphQL */ `
    mutation SetCapacity($setId: Id!, $capacity: Int!) {
      emoteSets { emoteSet(id: $setId) { capacity(capacity: $capacity) { id capacity } } }
    }
  `;
  await gqlAuthed(gql, { setId, capacity }, 'SetCapacity', accountId);
}

export async function deleteSet(setId: string, accountId?: string): Promise<void> {
  const gql = /* GraphQL */ `
    mutation DeleteSet($setId: Id!) { emoteSets { emoteSet(id: $setId) { delete } } }
  `;
  await gqlAuthed(gql, { setId }, 'DeleteSet', accountId);
}

/** Set (or clear, with null) the active emote set for a channel. */
export async function setActiveSet(ownerSeventvId: string, setId: string | null, accountId?: string): Promise<void> {
  const gql = /* GraphQL */ `
    mutation SetActiveSet($ownerId: Id!, $setId: Id) {
      users { user(id: $ownerId) { activeEmoteSet(emoteSetId: $setId) { id } } }
    }
  `;
  await gqlAuthed(gql, { ownerId: ownerSeventvId, setId }, 'SetActiveSet', accountId);
}

// Editor permission presets for the editor-management tab.
const fullEditorPermissions = (overrides?: Partial<{ manage: boolean; admin: boolean; manageEditors: boolean }>) => ({
  superAdmin: false,
  emoteSet: { admin: overrides?.admin ?? false, manage: overrides?.manage ?? true, create: overrides?.admin ?? false },
  emote: { admin: false, manage: false, create: false, transfer: false },
  user: {
    admin: false,
    manageBilling: false,
    manageProfile: false,
    manageEditors: overrides?.manageEditors ?? false,
    managePersonalEmoteSet: false,
  },
});

export async function addEditor(
  ownerSeventvId: string,
  editorSeventvId: string,
  perms?: { manage?: boolean; admin?: boolean; manageEditors?: boolean },
  accountId?: string,
): Promise<void> {
  const gql = /* GraphQL */ `
    mutation AddEditor($ownerId: Id!, $editorId: Id!, $permissions: UserEditorPermissionsInput!) {
      userEditors { create(userId: $ownerId, editorId: $editorId, permissions: $permissions) { editorId state } }
    }
  `;
  await gqlAuthed(
    gql,
    { ownerId: ownerSeventvId, editorId: editorSeventvId, permissions: fullEditorPermissions(perms) },
    'AddEditor',
    accountId,
  );
}

export async function updateEditorPermissions(
  ownerSeventvId: string,
  editorSeventvId: string,
  perms: { manage?: boolean; admin?: boolean; manageEditors?: boolean },
  accountId?: string,
): Promise<void> {
  const gql = /* GraphQL */ `
    mutation UpdateEditorPerms($ownerId: Id!, $editorId: Id!, $permissions: UserEditorPermissionsInput!) {
      userEditors { editor(userId: $ownerId, editorId: $editorId) { updatePermissions(permissions: $permissions) { editorId } } }
    }
  `;
  await gqlAuthed(
    gql,
    { ownerId: ownerSeventvId, editorId: editorSeventvId, permissions: fullEditorPermissions(perms) },
    'UpdateEditorPerms',
    accountId,
  );
}

export async function removeEditor(ownerSeventvId: string, editorSeventvId: string, accountId?: string): Promise<void> {
  const gql = /* GraphQL */ `
    mutation RemoveEditor($ownerId: Id!, $editorId: Id!) {
      userEditors { editor(userId: $ownerId, editorId: $editorId) { delete } }
    }
  `;
  await gqlAuthed(gql, { ownerId: ownerSeventvId, editorId: editorSeventvId }, 'RemoveEditor', accountId);
}

/**
 * Respond to a pending editor invite (the signed-in user is the editor). The
 * owner is the channel that invited you; the editor id is your own 7TV id.
 */
export async function respondToInvite(
  ownerSeventvId: string,
  myseventvId: string,
  accept: boolean,
  accountId?: string,
): Promise<void> {
  const gql = /* GraphQL */ `
    mutation RespondInvite($ownerId: Id!, $editorId: Id!, $state: UserEditorState!) {
      userEditors { editor(userId: $ownerId, editorId: $editorId) { updateState(state: $state) { state } } }
    }
  `;
  await gqlAuthed(
    gql,
    { ownerId: ownerSeventvId, editorId: myseventvId, state: accept ? 'ACCEPTED' : 'REJECTED' },
    'RespondInvite',
    accountId,
  );
}

// ── CDN helpers ──────────────────────────────────────────────────────────────

/** Best-quality animated WebP/AVIF URL for an emote id, for grid rendering. */
export function emoteCdnUrl(emoteId: string, scale: '1x' | '2x' | '3x' | '4x' = '3x'): string {
  return `https://cdn.7tv.app/emote/${emoteId}/${scale}.avif`;
}
