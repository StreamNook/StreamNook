// Compact in-composer quick-connect pill. Shows a small "Connect <platform>" pill
// when disconnected so you can connect right where you're trying to chat; renders
// nothing once connected. Disconnect (and full account management) lives in
// MultiChat Settings → Connections, so the composer stays uncluttered. Defaults to
// Kick for back-compat; pass `provider` for other platforms (YouTube, ...).
//
// Connecting from here runs the SAME flow as Settings > Profile > Accounts (via
// platformAccountStore), not a shortcut version of it.

import { PlatformLoginButton } from './PlatformLoginButton';
import { PROVIDERS, type ProviderId } from '../types/providers';

interface Props {
  connected: boolean;
  busy?: boolean;
  onConnect: () => void;
  provider?: ProviderId;
}

export function PlatformAccountChip({ connected, busy, onConnect, provider = 'kick' }: Props) {
  if (connected) return null;
  // The same button the empty states use, at its small size. This used to be a
  // brand-coloured outline with brand-coloured text, which is a different
  // treatment of the same action in a different corner of the app.
  return (
    <PlatformLoginButton
      provider={provider}
      onClick={onConnect}
      busy={busy}
      label={`Connect ${PROVIDERS[provider].label}`}
      size="sm"
    />
  );
}

export default PlatformAccountChip;
