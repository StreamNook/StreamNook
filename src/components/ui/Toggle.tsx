/**
 * The on/off switch used by every settings row.
 *
 * This exact markup was hand-copied into a dozen-plus files before it lived
 * here. Import it instead of re-declaring it, so a change to the switch lands
 * everywhere at once.
 */
interface ToggleProps {
  enabled: boolean;
  onChange: () => void;
  /** Announced to screen readers when the row's own label isn't adjacent. */
  ariaLabel?: string;
  disabled?: boolean;
}

export const Toggle = ({ enabled, onChange, ariaLabel, disabled = false }: ToggleProps) => {
  return (
  <button
    type="button"
    role="switch"
    aria-checked={enabled}
    aria-label={ariaLabel}
    disabled={disabled}
    onClick={onChange}
    className={`relative inline-flex h-6 w-11 items-center rounded-full transition-colors flex-shrink-0 ${
      enabled ? 'bg-accent' : 'bg-gray-600'
    } ${disabled ? 'opacity-50 cursor-not-allowed' : ''}`}
  >
    <span
      className={`inline-block h-4 w-4 transform rounded-full bg-white transition-transform ${
        enabled ? 'translate-x-6' : 'translate-x-1'
      }`}
    />
  </button>
  );
};

export default Toggle;
