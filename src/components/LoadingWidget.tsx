import { useState, useEffect, type JSX } from 'react';
import { IS_MOBILE } from '../utils/platform';
import PenroseMarch from './PenroseMarch';
import { Tooltip } from './ui/Tooltip';

// 7TV emote ids, each resolved from the 7TV API by popularity and fetched once to
// confirm it actually returns image bytes. A guessed id renders as a broken image
// in the middle of the loading screen, so if you add one here, verify it the same
// way rather than copying an id from memory.
const EMOTE_URLS: Record<string, string> = {
  // Waiting and anticipation, the emotions a loading screen is actually about.
  'Bedge': 'https://cdn.7tv.app/emote/01KYSFWRHRGDFWG9G4FRPQG7PP/1x.webp',
  'PauseChamp': 'https://cdn.7tv.app/emote/01KWTA6Y7ZWA8R0YV2PPSP8KHC/1x.webp',
  'Prayge': 'https://cdn.7tv.app/emote/01KYJZKBWX26V6ZN5TWXSDXMQA/1x.webp',
  'COPIUM': 'https://cdn.7tv.app/emote/01KZ9NMHCHSS9VC7T0BNEFZG4W/1x.webp',
  'monkaS': 'https://cdn.7tv.app/emote/01KXJ96QKM2CN3F1VBB78AZPXK/1x.webp',
  'Sadge': 'https://cdn.7tv.app/emote/01KZ75T5XV0ZAQBNEK65Y8CMMF/1x.webp',
  'Deadge': 'https://cdn.7tv.app/emote/01KXGJ965XGE4GMV0YBDQC06A9/1x.webp',
  'peepoLeave': 'https://cdn.7tv.app/emote/01KXKDF37AG61TWPAZSSF6GHQY/1x.webp',
  // Confusion and suspicion.
  'Aware': 'https://cdn.7tv.app/emote/01KY8KAB9R1GT2WF4MD5XEGBM3/1x.webp',
  'Clueless': 'https://cdn.7tv.app/emote/01KYTW5QR9RDPCWFYYZZQTG0ST/1x.webp',
  'Susge': 'https://cdn.7tv.app/emote/01KYD9RX2EG3741D0PTDJPK37K/1x.webp',
  'Erm': 'https://cdn.7tv.app/emote/01KZJ8GBZKGMBBP2FX52DWF9B1/1x.webp',
  'Hmm': 'https://cdn.7tv.app/emote/01KZMSZYMDK1X58E21DVMM9W37/1x.webp',
  'ThisIsFine': 'https://cdn.7tv.app/emote/01KYRB7C8WAC16XGGT8VF7MD2W/1x.webp',
  'NOOO': 'https://cdn.7tv.app/emote/01KYZBRWX4CPTR71YAGDEAX5T6/1x.webp',
  // Vibing while you wait.
  'catJAM': 'https://cdn.7tv.app/emote/01KZ2HZNS3024QY4WPPZXXGPAM/1x.webp',
  'vibePls': 'https://cdn.7tv.app/emote/01JT99E4YYE6GGV9V8GA9J8474/1x.webp',
  'Plink': 'https://cdn.7tv.app/emote/01KYFBMFJ3R56GYPNE3K3GE86G/1x.webp',
  'docSpin': 'https://cdn.7tv.app/emote/01EZPMWPER00077NX500A43YF8/1x.webp',
  'peepoHappy': 'https://cdn.7tv.app/emote/01KTR4A3Z08TPFNFA5CRVM9319/1x.webp',
  'Chatting': 'https://cdn.7tv.app/emote/01KZ7P94K3F7BBCZCZAE6BY8KX/1x.webp',
  'SNIFFA': 'https://cdn.7tv.app/emote/01KZ6VN4GMVFCKMCS7PPAMW2KX/1x.webp',
  // Payoff.
  'KEKW': 'https://cdn.7tv.app/emote/01KXHCAWV9XYKY15HP9RSDNC2V/1x.webp',
  'PepeLaugh': 'https://cdn.7tv.app/emote/01KCYPEMZ8MNNS6Q12PJHY4149/1x.webp',
  'GIGACHAD': 'https://cdn.7tv.app/emote/01KYE940CV1QMK2KVMZHRJAZ8W/1x.webp',
  'RIPBOZO': 'https://cdn.7tv.app/emote/01KYQ6K6CMW6RZG0WH1VFDV2EE/1x.webp',
  'WAYTOODANK': 'https://cdn.7tv.app/emote/01KQZMQM5AQK6PG7DFBAGF5V15/1x.webp',
  'Okayeg': 'https://cdn.7tv.app/emote/01KQW1GM9RZ9T2VVNGPP62KQ0D/1x.webp',
};

// Messages with emote placeholders
const FUNNY_MESSAGES = [
  // --- StreamNook ---
  'Twitch, made yours',
  'Nine cubes, one impossible triangle',
  'Bending the Penrose into place',
  'The triangle stays impossible',
  'Counting nine cubes, twice',
  'Assembling something that should not exist',
  'Your client, your rules',
  'Every emote provider, one chat',
  '7TV, BTTV and FFZ, all invited',
  'Paints on, badges up',
  'Chat, drops and clips in one window',
  'Tuned for the way you actually watch',
  'Built by someone who watches too much Twitch',
  'Made for the second monitor',

  // --- Twitch and streaming culture ---
  'Somewhere a streamer just hit go live',
  'Someone is about to clip this',
  'Raid inbound',
  'The chat is already typing',
  'Somebody just got their first sub',
  'Hype train departing shortly',
  'First message in chat, incoming',
  'The VOD will remember this',
  'Clip it and ship it',
  'Live is the best part',
  'Chat is the second screen',
  'Lurkers, we see you',
  'A lurker still counts as a viewer',
  'Go follow someone new tonight',
  'Somebody is streaming to four people and loving it',
  'The best moments are never planned',
  'Nine hours in and still going',
  'The subathon timer is still ticking',
  'Someone in chat is asking for a clip',
  'Mods are watching, be nice',
  'That one emote is about to get spammed',
  'The good clips come from the quiet nights',

  // --- Emotes, used for what they actually mean ---
  '{Bedge} still waiting',
  '{PauseChamp} any second now',
  '{Prayge} for good ping',
  '{COPIUM} it is almost ready',
  '{monkaS} the buffer is thinking',
  '{Sadge} the packets got lost',
  '{Deadge} loading took the L',
  '{peepoLeave} do not go, it is nearly there',
  '{Aware} this is taking a while',
  '{Clueless} ETA unknown',
  '{Susge} suspiciously slow',
  '{Erm} that is not supposed to happen',
  '{Hmm} interesting load times',
  '{ThisIsFine} everything is fine',
  '{NOOO} not another handshake',
  '{catJAM} vibing until it loads',
  '{vibePls} loading soundtrack playing',
  '{Plink} plinking through the queue',
  '{docSpin} spinning professionally',
  '{peepoHappy} almost there',
  '{Chatting} negotiating with the CDN',
  '{SNIFFA} sniffing out the stream',
  '{KEKW} the server said no',
  '{PepeLaugh} you thought it was ready',
  '{GIGACHAD} loading at maximum efficiency',
  '{RIPBOZO} rip the first attempt',
  '{WAYTOODANK} too fast to render',
  '{Okayeg} loading, eg',

  // --- Ctrl+K discovery, the feature nobody finds on their own ---
  'Ctrl+K opens the command palette, try it',
  'Everything in this app is one Ctrl+K away',
  'Ctrl+K searches streamers, settings and actions',
  'Press Ctrl+K sometime, thank us later',
  'Lost in the settings? Ctrl+K knows the way',

  // --- Timeless streamer and nerd humour ---
  'It is always DNS',
  'Blaming the router',
  'Waiting for the keyframe',
  'Chasing the live edge',
  'The handshake is taking its time',
  'Resolving every emote you own',
  'Waking the websocket',
  'Politely rate limiting ourselves',
  'Reading the manifest out loud',
  'Aligning the segments',
  'Measuring twice, loading once',
  'Any moment now, statistically',
  'Almost certainly nearly ready',
  'Yelling at the packets',
  'Turning it off and on again',
  'Pixels are on the way',
];

// Phones have no Ctrl key: keep the command palette tips desktop-only.
const MESSAGES = IS_MOBILE
  ? FUNNY_MESSAGES.filter((m) => !m.includes('Ctrl+K'))
  : FUNNY_MESSAGES;

interface LoadingWidgetProps {
  message?: string;
  useFunnyMessages?: boolean;
  showProxyNote?: boolean;
  fullScreen?: boolean;
  /** Mark width in CSS px. Default 100 (desktop). */
  size?: number;
}

const LoadingWidget = ({ message, useFunnyMessages = false, showProxyNote = false, fullScreen = true, size }: LoadingWidgetProps) => {
  const [currentMessageIndex, setCurrentMessageIndex] = useState(() =>
    Math.floor(Math.random() * MESSAGES.length)
  );

  useEffect(() => {
    if (!useFunnyMessages) return;

    const interval = setInterval(() => {
      // Pick a random message that's different from the current one
      setCurrentMessageIndex((prev) => {
        let newIndex;
        do {
          newIndex = Math.floor(Math.random() * MESSAGES.length);
        } while (newIndex === prev && MESSAGES.length > 1);
        return newIndex;
      });
    }, 10000); // Change message every 10 seconds

    return () => clearInterval(interval);
  }, [useFunnyMessages]);

  const displayMessage = useFunnyMessages
    ? MESSAGES[currentMessageIndex]
    : message || "Loading stream...";

  // Parse message and replace emote placeholders with images
  const renderMessage = (msg: string) => {
    const parts: (string | JSX.Element)[] = [];
    let lastIndex = 0;
    const emoteRegex = /\{(\w+)\}/g;
    let match;

    while ((match = emoteRegex.exec(msg)) !== null) {
      // Add text before the emote
      if (match.index > lastIndex) {
        parts.push(msg.substring(lastIndex, match.index));
      }

      // Add the emote image
      const emoteName = match[1];
      const emoteUrl = EMOTE_URLS[emoteName];

      if (emoteUrl) {
        parts.push(
          <Tooltip key={`${emoteName}-${match.index}`} content={emoteName} side="top">
            <img
              src={emoteUrl}
              alt={emoteName}
              className="inline-block w-6 h-6 mx-0.5 align-middle"
            />
          </Tooltip>
        );
      } else {
        // If emote URL not found, just show the name
        parts.push(emoteName);
      }

      lastIndex = match.index + match[0].length;
    }

    // Add remaining text
    if (lastIndex < msg.length) {
      parts.push(msg.substring(lastIndex));
    }

    return parts.length > 0 ? parts : msg;
  };

  return (
    <div className={`flex items-center justify-center ${fullScreen ? 'absolute inset-0 bg-black/70 backdrop-blur-sm' : 'h-full w-full'}`}>
      {/* Proxy/Ad-blocker note - positioned at top left */}
      {showProxyNote && (
        <div className="absolute top-4 left-4 max-w-xs animate-fade-in">
          <div className="glass-panel p-3 rounded-lg border border-accent/30 bg-background/80 backdrop-blur-md">
            <div className="flex items-start gap-2">
              <svg
                className="w-4 h-4 text-accent flex-shrink-0 mt-0.5"
                fill="none"
                stroke="currentColor"
                viewBox="0 0 24 24"
              >
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  strokeWidth={2}
                  d="M13 16h-1v-4h-1m1-4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z"
                />
              </svg>
              <div className="space-y-1">
                <p className="text-textPrimary text-xs font-medium">
                  Setting up ad-free playback
                </p>
                <p className="text-textSecondary text-xs leading-relaxed">
                  Loading may take a moment while we prepare your stream.
                </p>
                <p className="text-textMuted text-[10px] leading-relaxed">
                  If initial load times bother you, consider disabling the TTV LOL PRO plugin in Settings → Integrations.
                </p>
              </div>
            </div>
          </div>
        </div>
      )}

      <div className="flex flex-col items-center gap-6">
        {/* The StreamNook mark, sharing its geometry with the badges */}
        <PenroseMarch size={size} />

        <p className="text-textSecondary text-sm font-medium flex items-center">
          {useFunnyMessages ? renderMessage(displayMessage) : displayMessage}
        </p>
      </div>
    </div>
  );
};

export default LoadingWidget;
