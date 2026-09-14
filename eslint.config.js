import js from '@eslint/js';
import tseslint from 'typescript-eslint';
import reactHooks from 'eslint-plugin-react-hooks';
import reactRefresh from 'eslint-plugin-react-refresh';

export default tseslint.config(
  { ignores: ['dist', 'src-tauri', 'node_modules', 'analytics-dashboard'] },
  js.configs.recommended,
  ...tseslint.configs.recommended,
  {
    files: ['**/*.{ts,tsx}'],
    languageOptions: {
      ecmaVersion: 2020,
    },
    plugins: {
      'react-hooks': reactHooks,
      'react-refresh': reactRefresh,
    },
    rules: {
      // === LOGGER ENFORCEMENT ===
      // Ban raw console statements to enforce Logger usage
      'no-console': 'error',
      
      // === React Hook Rules ===
      ...reactHooks.configs.recommended.rules,
      'react-refresh/only-export-components': [
        'warn',
        { allowConstantExport: true },
      ],
      
      // === TypeScript Rules ===
      '@typescript-eslint/no-unused-vars': ['warn', { argsIgnorePattern: '^_' }],
      '@typescript-eslint/no-explicit-any': 'warn',
    },
  },
  // React Compiler rollout boundary. eslint-plugin-react-hooks 7.1 reports
  // every compiler-rule violation; inside the paths the compiler actually
  // compiles (vite.config.mjs REACT_COMPILER_SOURCES) they stay errors, so a
  // regression there fails the gate. Everywhere else they are warnings until
  // that path joins the rollout: visible, counted, not blocking.
  {
    files: ['**/*.{ts,tsx}'],
    ignores: [
      'src/components/ChatMessage.tsx',
      'src/components/ChatMessageList.tsx',
      'src/components/PlayerStatsOverlay.tsx',
      'src/components/chat/**',
      'src/components/multichat/**',
      'src/components/settings/**',
      'src/components/ui/**',
    ],
    rules: {
      'react-hooks/static-components': 'warn',
      'react-hooks/use-memo': 'warn',
      'react-hooks/preserve-manual-memoization': 'warn',
      'react-hooks/immutability': 'warn',
      'react-hooks/refs': 'warn',
      'react-hooks/set-state-in-effect': 'warn',
      'react-hooks/purity': 'warn',
      'react-hooks/set-state-in-render': 'warn',
      'react-hooks/error-boundaries': 'warn',
      'react-hooks/globals': 'warn',
    },
  },
  // Inside the settings directory but not yet in the rollout (mirrors
  // REACT_COMPILER_EXCLUDES in vite.config.mjs).
  {
    files: [
      'src/components/settings/ProfileSettings.tsx',
      'src/components/settings/ProfileOverview.tsx',
      'src/components/settings/PluginsSettings.tsx',
      'src/components/multichat/BlendedChatPane.tsx',
    ],
    rules: {
      'react-hooks/immutability': 'warn',
      'react-hooks/set-state-in-effect': 'warn',
      'react-hooks/refs': 'warn',
    },
  },
  // Allow console in these specific files (they wrap console intentionally)
  {
    files: ['src/utils/logger.ts', 'src/services/logService.ts'],
    rules: {
      'no-console': 'off',
    },
  },
);
