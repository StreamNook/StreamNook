// Known chat bots (lowercased logins), shared by the stream overlay's "Hide
// bots" filter and the chat widget's chat filters so the two surfaces agree on
// what a bot is. The bot BADGE check below catches channel-specific bots that
// carry one; this list only needs the well-known bots that don't.
export const KNOWN_BOTS = new Set([
  'nightbot', 'streamelements', 'streamlabs', 'moobot', 'fossabot', 'wizebot',
  'sery_bot', 'commanderroot', 'soundtrackbot', 'streamlootsbot', 'pretzelrocks',
  'tangiabot', 'blerp', 'kofistreambot', 'own3d', 'botrixoficial', 'coebot',
  'phantombot', 'thepositivebot', 'streamstickers', 'lattemotte',
  'restreambot', 'supibot', 'anotherttvviewer', 'streamdatabase', 'streamdbbot',
  // Command/utility bots that carry NO bot badge in the chat data (their "Chat Bot"
  // badge is Twitch web-client chrome, not sent over IRC), so only a name catches them.
  'potatbotat', 'pajbot', 'titlechange_bot', 'buttsbot', 'snusbot', 'deepbot',
  'ankhbot', 'vivbot', 'revlobot', 'dixperbro', 'botisimo', 'mikuia', 'wzbot',
  'own3dpro_bot', 'playwithviewersbot', 'thepixelbot', 'cloudbot', '9gag',
]);

// A bot badge. FrankerFaceZ (badge id 2), Chatterino, and Homies all label bot
// accounts with a badge titled exactly "Bot"; some Twitch/other sets say "Chat
// Bot". Match either, exact (not substring) so cosmetics like "Robot" or
// "Botany" don't trip it.
export const isChatBotBadge = (s?: string): boolean => {
  const v = (s || '').trim().toLowerCase();
  return v === 'bot' || v === 'chat bot';
};
