// Standalone on purpose: vite.config.mjs stays a pure build config with no
// vitest coupling. Tests run in the node environment (no jsdom) because the
// suite targets pure logic (key construction, slot identity, store guards);
// add an environment only when a test genuinely needs a DOM.
import { defineConfig } from 'vitest/config';

export default defineConfig({
  test: {
    environment: 'node',
    include: ['src/**/*.test.{ts,tsx}', 'test/**/*.test.{ts,tsx}'],
  },
});
