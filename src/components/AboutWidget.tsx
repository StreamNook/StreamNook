import { X, Github } from 'lucide-react';
import { invoke } from '@tauri-apps/api/core';
import { useEffect, useState } from 'react';
import { Tooltip } from './ui/Tooltip';

import { Logger } from '../utils/logger';
import streamnookLogo from '../assets/streamnook-logo.png';

interface AboutWidgetProps {
  onClose: () => void;
}

const AboutWidget = ({ onClose }: AboutWidgetProps) => {
  const [appVersion, setAppVersion] = useState('1.0.0');
  const [appName, setAppName] = useState('StreamNook');

  useEffect(() => {
    // Fetch app version from Cargo.toml
    invoke<string>('get_app_version')
      .then(version => setAppVersion(version))
      .catch(err => Logger.error('Failed to get app version:', err));

    invoke<string>('get_app_name')
      .then(name => setAppName(name))
      .catch(err => Logger.error('Failed to get app name:', err));
  }, []);

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 backdrop-blur-sm group">
      {/* Hover-sensitive background overlay */}
      <div
        className="absolute inset-0 group-hover:pointer-events-none"
        onClick={onClose}
      />

      <div className="glass-panel p-6 w-96 max-w-[90vw] shadow-2xl relative z-10">
        {/* Header */}
        <div className="flex items-center justify-between mb-4">
          <h2 className="text-lg font-bold text-textPrimary">About {appName}</h2>
          <Tooltip content="Close" side="top">
          <button
            onClick={onClose}
            className="p-1 text-textSecondary hover:text-textPrimary hover:bg-glass rounded transition-all duration-200"
          >
            <X size={18} />
          </button>
          </Tooltip>
        </div>

        {/* Content */}
        <div className="space-y-4 text-textSecondary text-sm">
          {/* Logo Section */}
          <div className="flex justify-center mb-4">
            <img
              src={streamnookLogo}
              alt="StreamNook"
              className="w-20 h-20 object-contain"
              draggable={false}
            />
          </div>

          {/* Description */}
          <div className="text-center">
            <p className="leading-relaxed text-textPrimary">
              A modern streaming companion for Twitch, Kick, and YouTube.
            </p>
          </div>

          {/* Made with love */}
          <div className="text-center py-2">
            <p className="text-textPrimary italic">
              Made by{' '}
              <a
                href="https://discord.com/users/681989594341834765"
                target="_blank"
                rel="noopener noreferrer"
                className="font-semibold hover:text-[#5865F2] transition-colors underline decoration-dotted"
              >
                Winters
              </a>{' '}
              with <span className="text-red-400">♥</span>
            </p>
          </div>

          {/* Special Thanks */}
          <div className="text-center py-2 px-4 bg-glass/30 rounded-lg">
            <p className="text-textSecondary text-xs mb-2">Special thanks and love to</p>
            <div className="flex items-center justify-center gap-3 flex-wrap">
              <Tooltip content="7TV" side="top">
              <a
                href="https://7tv.app"
                target="_blank"
                rel="noopener noreferrer"
                className="text-textPrimary hover:text-[#00A8FC] transition-colors font-medium text-sm"
              >
                7TV
              </a>
              </Tooltip>
              <span className="text-textSecondary">•</span>
              <Tooltip content="BetterTTV" side="top">
              <a
                href="https://betterttv.com"
                target="_blank"
                rel="noopener noreferrer"
                className="text-textPrimary hover:text-[#D50014] transition-colors font-medium text-sm"
              >
                BTTV
              </a>
              </Tooltip>
              <span className="text-textSecondary">•</span>
              <Tooltip content="FrankerFaceZ" side="top">
              <a
                href="https://www.frankerfacez.com"
                target="_blank"
                rel="noopener noreferrer"
                className="text-textPrimary hover:text-[#52A747] transition-colors font-medium text-sm"
              >
                FFZ
              </a>
              </Tooltip>

            </div>
          </div>

          {/* Tech Stack */}
          <div>
            <p className="text-textPrimary font-semibold mb-3 text-center">Built With</p>
            <div className="flex flex-wrap justify-center gap-2">
              {/* Tauri */}
              <span className="inline-flex items-center gap-1.5 px-3 py-1 bg-glass rounded-full text-xs">
                <svg className="w-3.5 h-3.5" viewBox="0 0 120 120" fill="none">
                  <path d="M88.5 28.5L31.5 28.5C31.5 41.4787 42.0213 52 55 52L65 52C77.9787 52 88.5 62.5213 88.5 75.5C88.5 88.4787 77.9787 99 65 99L55 99C42.0213 99 31.5 88.4787 31.5 75.5" stroke="#FFC131" strokeWidth="8" strokeLinecap="round" />
                </svg>
                <span className="text-[#FFC131] font-medium">Tauri</span>
              </span>

              {/* React */}
              <span className="inline-flex items-center gap-1.5 px-3 py-1 bg-glass rounded-full text-xs">
                <svg className="w-3.5 h-3.5" viewBox="0 0 24 24" fill="none">
                  <circle cx="12" cy="12" r="2" fill="#61DAFB" />
                  <ellipse cx="12" cy="12" rx="8" ry="3" stroke="#61DAFB" strokeWidth="1.5" fill="none" />
                  <ellipse cx="12" cy="12" rx="8" ry="3" stroke="#61DAFB" strokeWidth="1.5" fill="none" transform="rotate(60 12 12)" />
                  <ellipse cx="12" cy="12" rx="8" ry="3" stroke="#61DAFB" strokeWidth="1.5" fill="none" transform="rotate(120 12 12)" />
                </svg>
                <span className="text-[#61DAFB] font-medium">React</span>
              </span>

              {/* TypeScript */}
              <span className="inline-flex items-center gap-1.5 px-3 py-1 bg-glass rounded-full text-xs">
                <svg className="w-3.5 h-3.5" viewBox="0 0 24 24" fill="none">
                  <rect width="24" height="24" rx="3" fill="#3178C6" />
                  <path d="M14 8h3v1.5h-1.5V18H14V8zm-3.75 2.5V9H7v1.5h1.5V18h1.5v-7.5H10z" fill="white" />
                </svg>
                <span className="text-[#3178C6] font-medium">TypeScript</span>
              </span>

              {/* Rust */}
              <span className="inline-flex items-center gap-1.5 px-3 py-1 bg-glass rounded-full text-xs">
                <svg className="w-3.5 h-3.5" viewBox="0 0 24 24" fill="none">
                  <circle cx="12" cy="12" r="10" fill="#CE422B" />
                  <path d="M12 7l2.5 5h-5L12 7z M7 16h10" stroke="white" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" />
                </svg>
                <span className="text-[#CE422B] font-medium">Rust</span>
              </span>

              {/* Tailwind */}
              <span className="inline-flex items-center gap-1.5 px-3 py-1 bg-glass rounded-full text-xs">
                <svg className="w-3.5 h-3.5" viewBox="0 0 24 24" fill="none">
                  <path d="M12 6C9.33 6 7.67 7.33 7 10c1-1.33 2.17-1.83 3.5-1.5.76.19 1.3.74 1.9 1.35.98 1 2.12 2.15 4.6 2.15 2.67 0 4.33-1.33 5-4-1 1.33-2.17 1.83-3.5 1.5-.76-.19-1.3-.74-1.9-1.35C15.62 7.15 14.48 6 12 6zM7 12c-2.67 0-4.33 1.33-5 4 1-1.33 2.17-1.83 3.5-1.5.76.19 1.3.74 1.9 1.35.98 1 2.12 2.15 4.6 2.15 2.67 0 4.33-1.33 5-4-1 1.33-2.17 1.83-3.5 1.5-.76-.19-1.3-.74-1.9-1.35C10.62 13.15 9.48 12 7 12z" fill="#06B6D4" />
                </svg>
                <span className="text-[#06B6D4] font-medium">Tailwind</span>
              </span>


            </div>
          </div>

          {/* Version & Links */}
          <div className="pt-4 border-t border-borderSubtle">
            <div className="flex items-center justify-between">
              <p className="text-xs text-textSecondary">
                Version {appVersion}
              </p>
              <Tooltip content="View on GitHub" side="top">
              <a
                href="https://github.com/winters27"
                target="_blank"
                rel="noopener noreferrer"
                className="p-2 text-textSecondary hover:text-textPrimary hover:bg-glass rounded transition-all duration-200"
              >
                <Github size={18} />
              </a>
              </Tooltip>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
};

export default AboutWidget;
