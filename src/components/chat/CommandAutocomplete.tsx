import React, { useEffect, useRef } from 'react';

import { CommandDefinition } from '../../utils/chatCommands';

import { motion } from 'framer-motion';

interface CommandAutocompleteProps {
  /** List of matching commands to display */
  commands: CommandDefinition[];
  /** Currently selected index */
  selectedIndex: number;
  /** Callback when a command is selected */
  onSelect: (command: CommandDefinition) => void;
  /** Callback to change selected index */
  onSelectedIndexChange: (index: number) => void;
}

export const renderCommandUsage = (usage: string) => {
  const parts = usage.split(/(<[^>]+>|\[[^\]]+\])/);
  
  return (
    <>
      {parts.map((part, index) => {
        if (part.startsWith('<') && part.endsWith('>')) {
          const paramName = part.slice(1, -1);
          return (
            <span key={index} className="inline-flex items-center px-1.5 py-[1px] mx-1 rounded-[4px] bg-white/10 text-white shadow-sm border border-white/5 text-[10.5px] uppercase tracking-wide font-bold align-[1px]">
              {paramName}
            </span>
          );
        } else if (part.startsWith('[') && part.endsWith(']')) {
          const paramName = part.slice(1, -1);
          return (
            <span key={index} className="inline-flex items-center px-1.5 py-[1px] mx-1 rounded-[4px] bg-white/5 text-white/50 border border-white/5 text-[10.5px] uppercase tracking-wide font-semibold align-[1px]">
              {paramName}
            </span>
          );
        } else {
          return <span key={index}>{part}</span>;
        }
      })}
    </>
  );
};

const CommandItem: React.FC<{
  command: CommandDefinition;
  isSelected: boolean;
  onSelect: () => void;
  onHover: () => void;
  itemRef: (el: HTMLButtonElement | null) => void;
}> = ({ command, isSelected, onSelect, onHover, itemRef }) => {
  return (
    <button
      ref={itemRef}
      className={`relative w-full px-4 py-2.5 my-0.5 flex flex-col text-left transition-all duration-200 rounded-lg border overflow-hidden ${
        isSelected
          ? 'bg-white/5 border-white/10 shadow-[inset_0_0_20px_rgba(255,255,255,0.02)]'
          : 'border-transparent hover:bg-white/[0.03]'
      }`}
      onClick={onSelect}
      onMouseEnter={onHover}
    >
      <div className="w-full flex items-center gap-x-0.5 whitespace-nowrap overflow-visible">
        <span className="font-semibold text-white/90 flex flex-nowrap items-center gap-x-0.5">
          {renderCommandUsage(command.usage)}
        </span>
      </div>
      
      <div className="w-full flex items-center justify-between mt-1 pt-0.5 max-w-full">
        <span className="text-textSecondary text-[11px] opacity-80 truncate pr-2">
          {command.description}
        </span>
        <span className={`flex-shrink-0 text-[9px] font-semibold tracking-wider px-1.5 py-0.5 rounded-md uppercase ${
          command.hint
            ? 'bg-white/[0.04] text-white/35'
            : command.category === 'Custom'
              ? 'bg-amber-500/20 text-amber-300'
              : command.category === 'Moderator' || command.category === 'Chat Flow'
                ? 'bg-green-500/20 text-green-300'
                : command.category === 'Everyone'
                  ? 'bg-white/10 text-white/50'
                  : 'bg-purple-500/20 text-purple-300'
        }`}>
          {command.badgeLabel ?? command.category}
        </span>
      </div>
    </button>
  );
};

const CommandAutocomplete: React.FC<CommandAutocompleteProps> = ({
  commands,
  selectedIndex,
  onSelect,
  onSelectedIndexChange,
}) => {
  const listRef = useRef<HTMLDivElement>(null);
  const itemRefs = useRef<(HTMLButtonElement | null)[]>([]);

  // Scroll selected item into view
  useEffect(() => {
    const selectedItem = itemRefs.current[selectedIndex];
    if (selectedItem && listRef.current) {
      selectedItem.scrollIntoView({ block: 'nearest', behavior: 'smooth' });
    }
  }, [selectedIndex]);

  // Reset refs when commands change
  useEffect(() => {
    itemRefs.current = itemRefs.current.slice(0, commands.length);
  }, [commands.length]);

  if (commands.length === 0) {
    return null;
  }

  return (
    <motion.div
      initial={{ opacity: 0, y: 6, scale: 0.98 }}
      animate={{ opacity: 1, y: 0, scale: 1 }}
      exit={{ opacity: 0, y: 6, scale: 0.98 }}
      transition={{ duration: 0.15, ease: "easeOut" }}
      className="sn-popover absolute z-[60] bottom-full left-0 right-0 mb-2 h-[520px] max-h-[calc(100vh-120px)] flex flex-col overflow-hidden origin-bottom"
      ref={listRef as any}
    >
      {/* Header */}
      <div className="px-4 py-3 border-b border-white/5 flex justify-between items-center bg-background/[0.5] backdrop-blur-md shadow-sm z-10 relative">
        <span className="text-[11px] font-semibold text-white/50 uppercase tracking-wider">
          Chat Commands
        </span>
        <div className="flex items-center gap-1 opacity-60">
          <kbd className="px-1.5 py-0.5 rounded bg-white/10 text-[9px] font-mono text-white tracking-widest border border-white/10 shadow-sm leading-none">TAB</kbd>
          <span className="text-[10px] text-white">to select</span>
        </div>
      </div>
      
      {/* Scrollable Command List */}
      <div className="flex-1 overflow-y-auto custom-scrollbar p-1">
        {commands.map((cmd, index) => (
          <CommandItem
            key={cmd.name}
            command={cmd}
            isSelected={index === selectedIndex}
            onSelect={() => onSelect(cmd)}
            onHover={() => onSelectedIndexChange(index)}
            itemRef={(el) => { itemRefs.current[index] = el; }}
          />
        ))}
      </div>
    </motion.div>
  );
};

export default CommandAutocomplete;
