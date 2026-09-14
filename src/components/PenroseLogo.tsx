import { useState } from 'react';
import streamnookLogo from '../assets/streamnook-logo-128.webp';
import { Tooltip } from './ui/Tooltip';

interface PenroseLogoProps {
  onClick: () => void;
}

const PenroseLogo = ({ onClick }: PenroseLogoProps) => {
  const [isHovered, setIsHovered] = useState(false);

  return (
    <Tooltip content="About StreamNook" side="bottom">
      <button
        onClick={onClick}
        onMouseEnter={() => setIsHovered(true)}
        onMouseLeave={() => setIsHovered(false)}
        className="w-4 h-4 flex items-center justify-center cursor-pointer transition-transform duration-300 hover:scale-110"
      >
        <img
          src={streamnookLogo}
          alt="StreamNook"
          className={`w-4 h-4 object-contain transition-all duration-700 ${isHovered ? 'animate-penrose-rotate' : ''}`}
          style={{
            filter: isHovered ? 'drop-shadow(0 0 6px rgba(151, 177, 185, 0.5))' : 'none',
          }}
          draggable={false}
        />
      </button>
    </Tooltip>
  );
};

export default PenroseLogo;
