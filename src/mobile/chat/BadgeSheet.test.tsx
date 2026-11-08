import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it, vi } from 'vitest';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('../../utils/platform', () => ({ IS_MOBILE: true }));
vi.mock('../../stores/AppStore', () => ({ useAppStore: { getState: () => ({ currentUser: null }) } }));
const openRoute = vi.fn();
vi.mock('../../utils/badgeSpotlight', async (orig) => ({
  ...(await orig<typeof import('../../utils/badgeSpotlight')>()),
  openBadgeDetailsRoute: (...a: unknown[]) => openRoute(...a),
}));

import { BadgeSheetBody } from './BadgeSheet';
import { badgeCardFacts } from '../../hooks/useBadgeCard';
import { openBadgeInfo, useBadgeSpotlightStore } from '../../stores/badgeSpotlightStore';
import { twitchRowBadgeRef, type BadgeRef } from '../../utils/badgeSpotlight';

const ffz: BadgeRef = {
  provider: 'ffz',
  id: 'ffz-3',
  title: 'Supporter',
  art: [{ url: 'https://cdn.frankerfacez.com/badge/3/2/rounded', px: 36 }],
};

describe('phone badge sheet', () => {
  it('a tapped badge on the phone opens its sheet, never the desktop Badges page', () => {
    const sub = twitchRowBadgeRef('subscriber/12', { id: '12', title: '1-Year Subscriber', image_url_1x: 'https://x/1' });
    openBadgeInfo(sub);
    expect(useBadgeSpotlightStore.getState().badge).toBe(sub);
    expect(openRoute).not.toHaveBeenCalled();
    useBadgeSpotlightStore.getState().close();
  });

  it('shows the name, where it is from and what Rust knows', () => {
    const html = renderToStaticMarkup(
      <BadgeSheetBody
        facts={badgeCardFacts(
          ffz,
          {
            provider: 'ffz',
            provider_label: 'FrankerFaceZ',
            title: null,
            description: 'Supports FrankerFaceZ',
            image_1x: null,
            image_2x: null,
            image_4x: null,
            click_url: 'https://www.frankerfacez.com/subscribe',
            homepage: 'https://www.frankerfacez.com',
            holder_count: 1200,
            owned: true,
          },
          112,
        )}
      />,
    );
    expect(html).toContain('Supporter');
    expect(html).toContain('FrankerFaceZ');
    expect(html).toContain('Supports FrankerFaceZ');
    expect(html).toContain('1,200 holders');
    expect(html).toContain('You have this');
    expect(html).toContain('Open website');
  });

  it('a badge Rust knows nothing about still names its source', () => {
    const html = renderToStaticMarkup(<BadgeSheetBody facts={badgeCardFacts(ffz, null, 112)} />);
    expect(html).toContain('Supporter');
    expect(html).toContain('FrankerFaceZ');
    expect(html).not.toContain('Open website');
  });
});
