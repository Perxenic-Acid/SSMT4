<template>
  <svg class="ssmt4-mark" viewBox="0 0 64 64" role="img" aria-label="SSMT4">
    <defs>
      <linearGradient id="ssmt4-tile" x1="8" y1="5" x2="56" y2="59" gradientUnits="userSpaceOnUse">
        <stop stop-color="#202127" />
        <stop offset="0.55" stop-color="#111216" />
        <stop offset="1" stop-color="#090A0C" />
      </linearGradient>
      <linearGradient id="ssmt4-rim" x1="8" y1="4" x2="56" y2="60" gradientUnits="userSpaceOnUse">
        <stop stop-color="#F4F5F1" stop-opacity="0.42" />
        <stop offset="0.48" stop-color="#FFFFFF" stop-opacity="0.10" />
        <stop offset="1" stop-color="var(--ssmt-brand-accent)" stop-opacity="0.55" />
      </linearGradient>
      <linearGradient id="ssmt4-flow" x1="8" y1="32" x2="56" y2="32" gradientUnits="userSpaceOnUse">
        <stop stop-color="var(--ssmt-brand-accent)" stop-opacity="0" />
        <stop offset="0.35" stop-color="var(--ssmt-brand-accent)" stop-opacity="0.95" />
        <stop offset="0.62" stop-color="#FFFFFF" />
        <stop offset="1" stop-color="var(--ssmt-brand-accent)" stop-opacity="0" />
      </linearGradient>
      <linearGradient id="ssmt4-sheen" x1="0" y1="0" x2="1" y2="0">
        <stop stop-color="#FFFFFF" stop-opacity="0" />
        <stop offset="0.5" stop-color="#FFFFFF" stop-opacity="0.30" />
        <stop offset="1" stop-color="var(--ssmt-brand-accent)" stop-opacity="0" />
      </linearGradient>
      <filter id="ssmt4-glow" x="-40%" y="-40%" width="180%" height="180%">
        <feGaussianBlur stdDeviation="1.8" result="blur" />
        <feMerge>
          <feMergeNode in="blur" />
          <feMergeNode in="SourceGraphic" />
        </feMerge>
      </filter>
      <clipPath id="ssmt4-clip">
        <rect x="2" y="2" width="60" height="60" rx="16" />
      </clipPath>
    </defs>

    <rect x="2" y="2" width="60" height="60" rx="16" fill="url(#ssmt4-tile)" />
    <g clip-path="url(#ssmt4-clip)">
      <g transform="rotate(16 32 32)">
        <rect class="ssmt4-sheen" x="-20" y="-20" width="14" height="104" fill="url(#ssmt4-sheen)" />
      </g>
    </g>
    <rect x="2.75" y="2.75" width="58.5" height="58.5" rx="15.25" fill="none"
      stroke="url(#ssmt4-rim)" stroke-width="1.5" />

    <path class="ssmt4-s-base"
      d="M36.5 20H25.5C18.5 20 15 23.7 15 28.5C15 33.2 18.6 36 25.5 36H29C36 36 39.5 39 39.5 44C39.5 49 36 52 28 52H15" />
    <path class="ssmt4-s-flow" pathLength="100"
      d="M36.5 20H25.5C18.5 20 15 23.7 15 28.5C15 33.2 18.6 36 25.5 36H29C36 36 39.5 39 39.5 44C39.5 49 36 52 28 52H15" />

    <path class="ssmt4-node-base" d="M48 17L56 32L48 47L40 32Z" />
    <path class="ssmt4-node-cutout" d="M48 25.5L51.2 32L48 38.5L44.8 32Z" />
    <path class="ssmt4-node-flow" pathLength="100" d="M48 17L56 32L48 47L40 32Z" />
  </svg>
</template>

<style scoped>
.ssmt4-mark {
  display: block;
  width: 100%;
  height: 100%;
  overflow: visible;
}

.ssmt4-s-base,
.ssmt4-s-flow,
.ssmt4-node-flow {
  fill: none;
  stroke-linecap: round;
  stroke-linejoin: round;
}

.ssmt4-s-base {
  stroke: #f3f4f0;
  stroke-width: 5.4;
}

.ssmt4-s-flow {
  stroke: url(#ssmt4-flow);
  stroke-width: 2.8;
  stroke-dasharray: 15 85;
  animation: ssmt4-path-flow 2.8s linear infinite;
  filter: url(#ssmt4-glow);
}

.ssmt4-node-base {
  fill: var(--ssmt-brand-accent);
  stroke: var(--ssmt-brand-accent);
  stroke-width: 1.2;
  stroke-linejoin: round;
}

.ssmt4-node-cutout {
  fill: #101115;
}

.ssmt4-node-flow {
  stroke: url(#ssmt4-flow);
  stroke-width: 2.4;
  stroke-dasharray: 12 88;
  animation: ssmt4-path-flow 3.2s linear infinite -1.15s;
  filter: url(#ssmt4-glow);
}

.ssmt4-sheen {
  animation: ssmt4-sheen 4.2s cubic-bezier(.45, 0, .55, 1) infinite;
}

@keyframes ssmt4-path-flow {
  from { stroke-dashoffset: 100; }
  to { stroke-dashoffset: 0; }
}

@keyframes ssmt4-sheen {
  0%, 35% { transform: translateX(0); opacity: 0; }
  45% { opacity: 1; }
  70%, 100% { transform: translateX(112px); opacity: 0; }
}

@media (prefers-reduced-motion: reduce) {
  .ssmt4-s-flow,
  .ssmt4-node-flow,
  .ssmt4-sheen {
    animation: none;
  }

  .ssmt4-sheen {
    opacity: 0;
  }
}
</style>
