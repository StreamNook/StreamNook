// Asks Rust (category_tag_suggestions) for a category's tag choices: the chip
// row (an empty query) or the matches for typed text. Rust pools and ranks; this
// hook only keeps the newest answer, draws it while it is for the category on
// screen (`current` once it is for the text too), and asks once more when Rust
// says the category's own tags could not be read yet (once per revision, so a
// failing category never polls).
import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Logger } from '../utils/logger';
import {
  acceptAnswer,
  NO_RETRY,
  NO_TAGS,
  retryStep,
  tagsKey,
  tagView,
  type RetryState,
  type TagAnswer,
  type TagSuggestions,
  type TagView,
} from '../utils/tagFilter';

export interface TagSuggestionsAsk {
  gameName: string | null | undefined;
  query: string;
  selected: readonly string[];
  limit: number;
  /** Off while nothing shows the answer (a closed dropdown or sheet). */
  enabled: boolean;
  /** Anything that changes when the category's loaded rows change (a page
   *  counter, the rows array): Rust may know new tags then. */
  revision: unknown;
  /** The chip row: the hook keeps its last accepted row answer and sends it back
   *  as `shown`, so the row only ever grows while the screen lives. */
  row?: boolean;
}

export function useCategoryTagSuggestions(ask: TagSuggestionsAsk): TagView | null {
  const { query, selected, limit, enabled, revision, row = false } = ask;
  const name = ask.gameName ?? '';
  const [answer, setAnswer] = useState<TagAnswer | null>(null);
  const [retries, setRetries] = useState(0);
  const latest = useRef(0);
  const retry = useRef<RetryState>(NO_RETRY);
  const revisionId = useRef(0);
  const selectedRef = useRef<readonly string[]>(selected);
  const shownRef = useRef<readonly string[]>(NO_TAGS);

  // Layout effects run before the asking effect of the same commit, so an ask
  // never sends older picks or an older row. Another category's row is never
  // sent: the row answer must be for this name.
  useLayoutEffect(() => {
    selectedRef.current = selected;
    shownRef.current = row && answer && answer.gameName === name && answer.query === '' ? answer.labels : NO_TAGS;
  });

  const key = tagsKey(selected);

  // A new category, new picks or new rows: one more retry is allowed.
  useEffect(() => {
    revisionId.current += 1;
  }, [name, key, revision]);

  useEffect(() => {
    if (!enabled || !name) return;
    let live = true;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const reqId = ++latest.current;
    const revisionKey = String(revisionId.current);
    invoke<TagSuggestions>('category_tag_suggestions', {
      gameName: name,
      query,
      selected: [...selectedRef.current],
      shown: [...shownRef.current],
      limit,
    })
      .then((got) => {
        if (!live || !acceptAnswer(reqId, latest.current)) return;
        setAnswer({ ...got, gameName: name, query });
        const step = retryStep(retry.current, revisionKey, got);
        retry.current = step.state;
        if (step.delay !== null) timer = setTimeout(() => setRetries((n) => n + 1), step.delay);
      })
      .catch((err: unknown) => Logger.warn('[TagSuggestions] category_tag_suggestions failed:', err));
    return () => {
      live = false;
      if (timer) clearTimeout(timer);
    };
    // `key` names the picks and `revision`/`retries` are triggers; the values
    // themselves are read through the refs above.
  }, [enabled, name, query, key, limit, revision, retries]);

  return tagView(answer, name, query);
}
