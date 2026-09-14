import { useState } from 'react';
import { Plus, Trash2, Check, Pencil } from 'lucide-react';
import { useAppStore } from '../../stores/AppStore';
import { BUILT_IN_COMPACT_PRESETS, DEFAULT_COMPACT_PRESET_ID } from '../../constants/compactViewPresets';
import { Tooltip } from '../ui/Tooltip';
import { SettingsSection } from './_primitives';
import type { CompactViewPreset } from '../../types';

const CompactViewSettings = () => {
  const { settings, updateSettings } = useAppStore();
  
  // Get current settings with defaults
  const compactViewSettings = settings.compact_view ?? {
    selectedPresetId: DEFAULT_COMPACT_PRESET_ID,
    customPresets: []
  };
  
  const [isAddingCustom, setIsAddingCustom] = useState(false);
  const [editingPresetId, setEditingPresetId] = useState<string | null>(null);
  const [customName, setCustomName] = useState('');
  const [customWidth, setCustomWidth] = useState('1280');
  const [customHeight, setCustomHeight] = useState('720');

  const handleSelectPreset = async (presetId: string) => {
    await updateSettings({
      ...settings,
      compact_view: {
        ...compactViewSettings,
        selectedPresetId: presetId
      }
    });
  };

  const handleAddCustomPreset = async () => {
    const width = parseInt(customWidth, 10);
    const height = parseInt(customHeight, 10);
    
    if (isNaN(width) || isNaN(height) || width < 400 || height < 300) {
      return; // Basic validation
    }
    
    const newPreset: CompactViewPreset = {
      id: `custom-${Date.now()}`,
      name: customName.trim() || `Custom ${width}x${height}`,
      width,
      height,
      isBuiltIn: false
    };
    
    const newCustomPresets = [...(compactViewSettings.customPresets || []), newPreset];
    
    await updateSettings({
      ...settings,
      compact_view: {
        ...compactViewSettings,
        customPresets: newCustomPresets,
        selectedPresetId: newPreset.id // Auto-select the new preset
      }
    });
    
    // Reset form
    setIsAddingCustom(false);
    setCustomName('');
    setCustomWidth('1280');
    setCustomHeight('720');
  };

  const handleDeleteCustomPreset = async (presetId: string) => {
    const newCustomPresets = compactViewSettings.customPresets.filter(p => p.id !== presetId);
    
    // If deleting the selected preset, fall back to default
    const newSelectedId = compactViewSettings.selectedPresetId === presetId 
      ? DEFAULT_COMPACT_PRESET_ID 
      : compactViewSettings.selectedPresetId;
    
    await updateSettings({
      ...settings,
      compact_view: {
        ...compactViewSettings,
        customPresets: newCustomPresets,
        selectedPresetId: newSelectedId
      }
    });
  };

  const handleUpdateCustomPreset = async (presetId: string) => {
    const width = parseInt(customWidth, 10);
    const height = parseInt(customHeight, 10);
    
    if (isNaN(width) || isNaN(height) || width < 400 || height < 300) {
      return;
    }
    
    const newCustomPresets = compactViewSettings.customPresets.map(p => {
      if (p.id === presetId) {
        return {
          ...p,
          name: customName.trim() || `Custom ${width}x${height}`,
          width,
          height
        };
      }
      return p;
    });
    
    await updateSettings({
      ...settings,
      compact_view: {
        ...compactViewSettings,
        customPresets: newCustomPresets
      }
    });
    
    setEditingPresetId(null);
    setCustomName('');
    setCustomWidth('1280');
    setCustomHeight('720');
  };

  const startEditingPreset = (preset: CompactViewPreset) => {
    setEditingPresetId(preset.id);
    setCustomName(preset.name);
    setCustomWidth(preset.width.toString());
    setCustomHeight(preset.height.toString());
  };

  const cancelEditing = () => {
    setEditingPresetId(null);
    setIsAddingCustom(false);
    setCustomName('');
    setCustomWidth('1280');
    setCustomHeight('720');
  };

  return (
    <SettingsSection
      label="Compact View"
      description="Pick the window size StreamNook snaps to when you enter Compact View, handy for parking it on a second monitor."
      bare
    >
      <div className="space-y-2">
        <label className="text-sm text-textPrimary font-medium">Preset Sizes</label>
        <div className="grid grid-cols-2 gap-2">
          {BUILT_IN_COMPACT_PRESETS.map((preset) => (
            <button
              key={preset.id}
              onClick={() => handleSelectPreset(preset.id)}
              className={`p-3 rounded-lg border transition-all text-left ${
                compactViewSettings.selectedPresetId === preset.id
                  ? 'border-accent bg-accent/10'
                  : 'border-borderSubtle hover:border-border'
              }`}
            >
              <div className="flex items-center justify-between">
                <span className={`text-sm font-medium ${
                  compactViewSettings.selectedPresetId === preset.id ? 'text-accent' : 'text-textPrimary'
                }`}>
                  {preset.name}
                </span>
                {compactViewSettings.selectedPresetId === preset.id && (
                  <Check size={16} className="text-accent" />
                )}
              </div>
              <span className="text-xs text-textMuted">{preset.width} × {preset.height}</span>
            </button>
          ))}
        </div>
      </div>

      {/* Custom Presets */}
      {compactViewSettings.customPresets && compactViewSettings.customPresets.length > 0 && (
        <div className="space-y-2">
          <label className="text-sm text-textPrimary font-medium">Custom Presets</label>
          <div className="space-y-2">
            {compactViewSettings.customPresets.map((preset) => (
              <div key={preset.id}>
                {editingPresetId === preset.id ? (
                  // Editing mode
                  <div className="p-3 rounded-lg border border-accent bg-accent/5 space-y-3">
                    <div>
                      <label className="text-xs text-textMuted mb-1 block">Name (optional)</label>
                      <input
                        type="text"
                        value={customName}
                        onChange={(e) => setCustomName(e.target.value)}
                        placeholder="Second monitor"
                        className="w-full px-3 py-2 rounded-md bg-surface border border-borderSubtle text-sm text-textPrimary placeholder:text-textMuted focus:outline-none focus:border-accent"
                      />
                    </div>
                    <div className="flex gap-2">
                      <div className="flex-1">
                        <label className="text-xs text-textMuted mb-1 block">Width</label>
                        <input
                          type="number"
                          value={customWidth}
                          onChange={(e) => setCustomWidth(e.target.value)}
                          min={400}
                          max={3840}
                          className="w-full px-3 py-2 rounded-md bg-surface border border-borderSubtle text-sm text-textPrimary focus:outline-none focus:border-accent"
                        />
                      </div>
                      <div className="flex-1">
                        <label className="text-xs text-textMuted mb-1 block">Height</label>
                        <input
                          type="number"
                          value={customHeight}
                          onChange={(e) => setCustomHeight(e.target.value)}
                          min={300}
                          max={2160}
                          className="w-full px-3 py-2 rounded-md bg-surface border border-borderSubtle text-sm text-textPrimary focus:outline-none focus:border-accent"
                        />
                      </div>
                    </div>
                    <div className="flex justify-end gap-2">
                      <button
                        onClick={cancelEditing}
                        className="px-3 py-1.5 rounded-md text-sm text-textSecondary hover:text-textPrimary hover:bg-glass transition-colors"
                      >
                        Cancel
                      </button>
                      <button
                        onClick={() => handleUpdateCustomPreset(preset.id)}
                        className="px-3 py-1.5 rounded-md text-sm bg-accent text-white hover:bg-accent/90 transition-colors"
                      >
                        Save
                      </button>
                    </div>
                  </div>
                ) : (
                  // Display mode
                  <div
                    className={`p-3 rounded-lg border transition-all flex items-center justify-between ${
                      compactViewSettings.selectedPresetId === preset.id
                        ? 'border-accent bg-accent/10'
                        : 'border-borderSubtle hover:border-border'
                    }`}
                  >
                    <button
                      onClick={() => handleSelectPreset(preset.id)}
                      className="flex-1 text-left"
                    >
                      <div className="flex items-center gap-2">
                        <span className={`text-sm font-medium ${
                          compactViewSettings.selectedPresetId === preset.id ? 'text-accent' : 'text-textPrimary'
                        }`}>
                          {preset.name}
                        </span>
                        {compactViewSettings.selectedPresetId === preset.id && (
                          <Check size={14} className="text-accent" />
                        )}
                      </div>
                      <span className="text-xs text-textMuted">{preset.width} × {preset.height}</span>
                    </button>
                    <div className="flex gap-1">
                      <Tooltip content="Edit preset" side="top">
                      <button
                        onClick={() => startEditingPreset(preset)}
                        className="p-1.5 rounded-md text-textMuted hover:text-textPrimary hover:bg-glass transition-colors"
                      >
                        <Pencil size={14} />
                      </button>
                      </Tooltip>
                      <Tooltip content="Delete preset" side="top">
                      <button
                        onClick={() => handleDeleteCustomPreset(preset.id)}
                        className="p-1.5 rounded-md text-textMuted hover:text-red-400 hover:bg-glass transition-colors"
                      >
                        <Trash2 size={14} />
                      </button>
                      </Tooltip>
                    </div>
                  </div>
                )}
              </div>
            ))}
          </div>
        </div>
      )}

      {/* Add Custom Preset */}
      {isAddingCustom ? (
        <div className="p-3 rounded-lg border border-accent bg-accent/5 space-y-3">
          <div>
            <label className="text-xs text-textMuted mb-1 block">Name (optional)</label>
            <input
              type="text"
              value={customName}
              onChange={(e) => setCustomName(e.target.value)}
              placeholder="Second monitor"
              className="w-full px-3 py-2 rounded-md bg-surface border border-borderSubtle text-sm text-textPrimary placeholder:text-textMuted focus:outline-none focus:border-accent"
            />
          </div>
          <div className="flex gap-2">
            <div className="flex-1">
              <label className="text-xs text-textMuted mb-1 block">Width</label>
              <input
                type="number"
                value={customWidth}
                onChange={(e) => setCustomWidth(e.target.value)}
                min={400}
                max={3840}
                className="w-full px-3 py-2 rounded-md bg-surface border border-borderSubtle text-sm text-textPrimary focus:outline-none focus:border-accent"
              />
            </div>
            <div className="flex-1">
              <label className="text-xs text-textMuted mb-1 block">Height</label>
              <input
                type="number"
                value={customHeight}
                onChange={(e) => setCustomHeight(e.target.value)}
                min={300}
                max={2160}
                className="w-full px-3 py-2 rounded-md bg-surface border border-borderSubtle text-sm text-textPrimary focus:outline-none focus:border-accent"
              />
            </div>
          </div>
          <div className="flex justify-end gap-2">
            <button
              onClick={cancelEditing}
              className="px-3 py-1.5 rounded-md text-sm text-textSecondary hover:text-textPrimary hover:bg-glass transition-colors"
            >
              Cancel
            </button>
            <button
              onClick={handleAddCustomPreset}
              className="px-3 py-1.5 rounded-md text-sm bg-accent text-white hover:bg-accent/90 transition-colors"
            >
              Add Preset
            </button>
          </div>
        </div>
      ) : (
        <button
          onClick={() => setIsAddingCustom(true)}
          className="w-full p-3 rounded-lg border border-dashed border-borderSubtle hover:border-accent hover:bg-accent/5 transition-all flex items-center justify-center gap-2 text-sm text-textSecondary hover:text-accent"
        >
          <Plus size={16} />
          Add Custom Preset
        </button>
      )}

      {/* Info text */}
      <div className="p-3 rounded-lg bg-surface/50 border border-borderSubtle space-y-1">
        <p className="text-xs text-textMuted">
          Sizes are the whole window. StreamNook allows for the window borders on its own and keeps the video at 16:9.
        </p>
        <p className="text-xs text-textMuted">
          So a 1080px preset gives you a window 1080px wide, with the video a touch smaller to leave room for the borders.
        </p>
      </div>
    </SettingsSection>
  );
};

export default CompactViewSettings;
