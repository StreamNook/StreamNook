import { useState, useRef, useEffect } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import { ChevronDown, Check } from 'lucide-react';

export interface DropdownOption {
    value: string;
    label: string;
}

export interface GlassSelectProps {
    value: string;
    onChange: (value: string) => void;
    options: DropdownOption[];
    className?: string; // Additional classes for the trigger
    placement?: 'top' | 'bottom'; // Controls dropdown open direction
}

export const GlassSelect = ({ value, onChange, options, className = '', placement = 'bottom' }: GlassSelectProps) => {
    const [isOpen, setIsOpen] = useState(false);
    const containerRef = useRef<HTMLDivElement>(null);

    const activeOption = options.find(o => o.value === value) || options[0];

    useEffect(() => {
        const handleClickOutside = (e: MouseEvent) => {
            if (containerRef.current && !containerRef.current.contains(e.target as Node)) {
                setIsOpen(false);
            }
        };

        if (isOpen) {
            document.addEventListener('mousedown', handleClickOutside);
        }

        return () => {
            document.removeEventListener('mousedown', handleClickOutside);
        };
    }, [isOpen]);

    return (
        <div className="relative inline-block text-left" ref={containerRef}>
            <button
                type="button"
                onClick={() => setIsOpen(!isOpen)}
                className={`glass-input !rounded-lg outline-none text-sm text-textPrimary font-medium transition-all px-3 py-1.5 flex items-center justify-between gap-3 min-w-[130px] cursor-pointer ${className} ${isOpen ? 'ring-1 ring-accent bg-white/10' : ''}`}
            >
                <span className="truncate">{activeOption?.label || 'Select...'}</span>
                <ChevronDown 
                    size={14} 
                    className={`text-textSecondary shrink-0 transition-transform duration-300 ${isOpen ? '-rotate-180 text-accent' : ''}`} 
                />
            </button>

            <AnimatePresence>
                {isOpen && (
                    <motion.div
                        initial={{ opacity: 0, scale: 0.95, y: placement === 'top' ? 5 : -5 }}
                        animate={{ opacity: 1, scale: 1, y: 0 }}
                        exit={{ opacity: 0, scale: 0.95, y: placement === 'top' ? 5 : -5 }}
                        transition={{ duration: 0.15, ease: 'easeOut' }}
                        className={`absolute right-0 z-[100] w-full min-w-[150px] rounded-lg bg-glass-ink/90 backdrop-blur-3xl shadow-[0_8px_32px_rgba(0,0,0,0.6)] border border-white/10 overflow-hidden ${
                            placement === 'top' ? 'bottom-full mb-1 origin-bottom-right' : 'mt-1 origin-top-right'
                        }`}
                    >
                        <div className="py-1 flex flex-col w-full">
                            {options.map((option) => (
                                <button
                                    key={option.value}
                                    onClick={() => {
                                        onChange(option.value);
                                        setIsOpen(false);
                                    }}
                                    className={`w-full text-left px-3 py-2 text-sm flex items-center justify-between transition-colors outline-none
                                        ${value === option.value 
                                            ? 'text-accent font-bold bg-white/5' 
                                            : 'text-textPrimary hover:bg-white/10 hover:text-white focus:bg-white/10'
                                        }
                                    `}
                                >
                                    <span className="truncate">{option.label}</span>
                                    {value === option.value && <Check size={14} className="text-accent shrink-0 ml-2" />}
                                </button>
                            ))}
                        </div>
                    </motion.div>
                )}
            </AnimatePresence>
        </div>
    );
};
