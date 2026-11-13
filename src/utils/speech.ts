// Text to speech for the page: automations' Speak action and `/tts`.
//
// Desktop (and Linux's CEF page) speak through the page's `speechSynthesis`;
// Android goes through the app command `speak_android` to its TextToSpeech
// plugin (the WebView's own synthesis is unreliable there). Every path answers
// whether the line was spoken: false means "show the text instead" (no voice,
// the plugin answered `{ spoken: false }`, a Linux build with no voices).
import { invoke } from '@tauri-apps/api/core';
import { IS_ANDROID } from './platform';

/** The longest line spoken, the same 300 as the Speak action. */
export const MAX_SPOKEN_CHARS = 300;

/** A line as it is spoken: one trimmed line of at most 300 characters; null for nothing to say. */
export function spokenLine(text: string): string | null {
  const line = text.replace(/\s+/g, ' ').trim().slice(0, MAX_SPOKEN_CHARS);
  return line === '' ? null : line;
}

/** Voices the page's synthesis has. Linux lists them late (`voiceschanged`), so wait up to a second once. */
function hasVoices(synth: SpeechSynthesis): Promise<boolean> {
  if (synth.getVoices().length > 0) return Promise.resolve(true);
  return new Promise<boolean>((resolve) => {
    const finish = () => {
      synth.removeEventListener('voiceschanged', finish);
      window.clearTimeout(timer);
      resolve(synth.getVoices().length > 0);
    };
    const timer = window.setTimeout(finish, 1000);
    synth.addEventListener('voiceschanged', finish);
  });
}

let probe: Promise<boolean> | null = null;

/** Whether this page can speak. Probed once at start (Linux builds with no voices stay silent and show text);
 *  a "no" is probed again later, since voices can be installed. Android answers per line. */
export function speechAvailable(): Promise<boolean> {
  if (IS_ANDROID) return Promise.resolve(true);
  const synth = typeof window !== 'undefined' ? window.speechSynthesis : undefined;
  if (!synth) return Promise.resolve(false);
  probe ??= hasVoices(synth).then((ok) => {
    if (!ok) probe = null;
    return ok;
  });
  return probe;
}

/** Speak a line; true when it was handed to the speech engine. */
export async function speakText(text: string): Promise<boolean> {
  const line = spokenLine(text);
  if (!line) return false;
  if (IS_ANDROID) {
    try {
      const answer = await invoke<{ spoken: boolean }>('speak_android', { text: line });
      return answer.spoken === true;
    } catch {
      return false;
    }
  }
  if (!(await speechAvailable())) return false;
  window.speechSynthesis.speak(new SpeechSynthesisUtterance(line));
  return true;
}
