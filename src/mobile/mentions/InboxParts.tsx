// The pieces an inbox row and the arrival card share: the sender's face with
// the kind's mark on it, and a message with its emotes.
import React, { useState } from 'react';
import { AtSign, Highlighter, MessageCircle, Reply } from 'lucide-react';
import type { MentionEntry, MentionPart } from '../../types';
import type { FaceInfo } from './inboxModel';

const KIND_ICON = { mention: AtSign, reply: Reply, word: Highlighter, whisper: MessageCircle } as const;

/** The sender's picture (Rust's cached lookup), or their initial on their own
 *  color while it loads or when there is none, with the kind's mark on its
 *  corner. */
export const InboxFace: React.FC<{ face: FaceInfo; avatar: string | null; size: number; dim?: boolean }> = ({
  face,
  avatar,
  size,
  dim = false,
}) => {
  const [broken, setBroken] = useState<string | null>(null);
  const Icon = KIND_ICON[face.kind] ?? AtSign;
  const color = face.color || 'var(--color-accent)';
  const chip = Math.round(size * 0.5);
  const showPicture = !!avatar && broken !== avatar;
  return (
    <span className="relative shrink-0 block" style={{ width: size, height: size }}>
      <span className={`block w-full h-full rounded-full overflow-hidden ${dim ? 'opacity-60' : ''}`}>
        {showPicture ? (
          <img
            src={avatar}
            alt=""
            draggable={false}
            className="w-full h-full object-cover"
            onError={() => setBroken(avatar)}
          />
        ) : (
          <span
            className="w-full h-full flex items-center justify-center font-semibold text-white"
            style={{
              fontSize: size * 0.42,
              background: `linear-gradient(135deg, ${color}, color-mix(in srgb, ${color} 55%, #000))`,
            }}
          >
            {(face.name || '?').charAt(0).toUpperCase()}
          </span>
        )}
      </span>
      <span
        className="sn-inbox-kind absolute -right-1 -bottom-1 rounded-full flex items-center justify-center"
        style={{ width: chip, height: chip }}
        aria-hidden
      >
        <Icon size={Math.round(chip * 0.62)} strokeWidth={2.7} />
      </span>
    </span>
  );
};

/** Words and emotes in order, emotes at the text's height; the plain text when
 *  there are no parts. */
export const InboxParts: React.FC<{ parts: MentionPart[]; text: string; emote: number }> = ({ parts, text, emote }) => {
  if (!parts.length) return <>{text}</>;
  return (
    <>
      {parts.map((p, i) =>
        p.type === 'emote' ? (
          <img
            key={i}
            src={p.url}
            alt={p.name}
            title={p.name}
            draggable={false}
            className="inline-block align-middle -my-1 mx-px object-contain"
            style={{ height: emote, maxWidth: emote * 3 }}
          />
        ) : (
          <React.Fragment key={i}>{p.text}</React.Fragment>
        ),
      )}
    </>
  );
};

/** A mention as the chat drew it, or the line for one a moderator deleted. */
export const InboxMessage: React.FC<{ entry: MentionEntry; emote: number }> = ({ entry, emote }) => {
  if (entry.removed) return <span className="italic text-textMuted">Message deleted by a moderator</span>;
  return <InboxParts parts={entry.parts ?? []} text={entry.text} emote={emote} />;
};
