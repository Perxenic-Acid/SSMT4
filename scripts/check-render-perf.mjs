#!/usr/bin/env node
/* ═══════════════════════════════════════════════════════════════════
   渲染性能回归检查（render performance lint）

   背景：工作区页面曾因"逐列表项 backdrop-filter"触发 Chromium(WebView2)
   合成器 bug —— 滚动时背景闪烁、文字消失。全项目排查修复后，用本脚本
   防止同类问题回流。规则详见 docs/ui-glass-design.md 末尾两节铁律。

   检查项：
     1. backdrop-filter 出现在逐列表项/逐节点/超大容器选择器上
     2. infinite 动画中包含绘制/布局属性（box-shadow/left/...）
     3. will-change 声明 box-shadow/filter（常驻多余提升层）

   用法：node scripts/check-render-perf.mjs   （退出码非 0 表示有违规）
   ═══════════════════════════════════════════════════════════════════ */

import { readFileSync, readdirSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';

const SRC_DIR = fileURLToPath(new URL('../src', import.meta.url));

/* ── 已审阅并接受的白名单（必须带注释说明理由） ─────────────────────
   crystalPulse：仅用于 GamesDrawer 游戏图标（≤15 个有界元素，
   抽屉关闭即销毁 v-if），两个伪元素已被占用无法无侵入改造。 */
const ACCEPTED = new Set(['GamesDrawer.vue::crystalPulse']);

/* 逐列表项/逐节点特征选择器片段（命中即视为高风险） */
const PER_ITEM_SELECTOR_HINTS = [
  'mod-card', 'card-info', 'badge', 'subgroup-card', 'list-item',
  'texture-item', 'bp-node', 'mod-node', 'vis-label', 'node-badge',
  'img-toggle', 'img-eye', 'row-move-btn', 'delete-btn', 'glass-table',
  'inner-panel', 'blueprint-workspace', 'entry', 'item-action',
];

/* 动画中的绘制/布局属性（每帧重绘或重排） */
const PAINT_LAYOUT_PROPS = new Set([
  'box-shadow', 'border-color', 'border-width', 'left', 'top', 'right',
  'bottom', 'width', 'height', 'margin', 'margin-top', 'margin-left',
  'padding', 'padding-top', 'background', 'background-color', 'background-position', 'filter',
  'clip-path', 'font-size', 'outline', 'text-shadow',
]);

const issues = [];

function* walk(dir) {
  for (const entry of readdirSync(dir)) {
    const full = join(dir, entry);
    if (statSync(full).isDirectory()) yield* walk(full);
    else if (/\.(vue|css)$/.test(entry)) yield full;
  }
}

/** 按花括号配对提取 CSS 规则块 */
function extractBlocks(css) {
  const blocks = [];
  const re = /([^{}]+)\{/g;
  let m;
  while ((m = re.exec(css))) {
    let depth = 1;
    let i = re.lastIndex;
    while (i < css.length && depth > 0) {
      if (css[i] === '{') depth++;
      else if (css[i] === '}') depth--;
      i++;
    }
    blocks.push({ selector: m[1].trim(), body: css.slice(re.lastIndex, i - 1) });
    re.lastIndex = i;
  }
  return blocks;
}

for (const file of walk(SRC_DIR)) {
  const rel = relative(SRC_DIR, file).replaceAll('\\', '/');
  const name = rel.split('/').pop();
  const raw = readFileSync(file, 'utf8');

  // 去掉注释，避免注释文字误报
  const css = raw.replace(/\/\*[\s\S]*?\*\//g, '');

  /* ── 检查 1 & 3：逐规则扫描 backdrop-filter / will-change ── */
  for (const { selector, body } of extractBlocks(css)) {
    const selNorm = selector.replace(/\s+/g, ' ');
    const perItem = PER_ITEM_SELECTOR_HINTS.some((h) => selNorm.includes(h));

    const bf = /(?<!-)backdrop-filter\s*:\s*([^;}!]+)/.exec(body);
    if (bf && bf[1].trim() !== 'none' && perItem) {
      issues.push(`[backdrop-filter] ${rel} :: ${selNorm.slice(0, 80)} -> ${bf[1].trim()}`);
    }

    const wc = /will-change\s*:\s*([^;}]+)/.exec(body);
    if (wc && /box-shadow|(?<!-)filter/.test(wc[1])) {
      issues.push(`[will-change]    ${rel} :: ${selNorm.slice(0, 80)} -> ${wc[1].trim()}`);
    }
  }

  /* ── 检查 2：infinite 动画的绘制/布局属性 ── */
  const kfRe = /@keyframes\s+([\w-]+)\s*\{/g;
  let km;
  while ((km = kfRe.exec(css))) {
    let depth = 1;
    let i = kfRe.lastIndex;
    while (i < css.length && depth > 0) {
      if (css[i] === '{') depth++;
      else if (css[i] === '}') depth--;
      i++;
    }
    const name = km[1];
    const body = css.slice(kfRe.lastIndex, i - 1);
    kfRe.lastIndex = i;

    const infinite = new RegExp(`animation:[^;]*${name}[^;]*infinite`).test(css);
    if (!infinite) continue;
    if (ACCEPTED.has(`${name}::${name}`) || ACCEPTED.has(`${name}`) || ACCEPTED.has(`${rel.split('/').pop()}::${name}`)) continue;

    const props = new Set([...body.matchAll(/([a-z-]+)\s*:/g)].map((x) => x[1]));
    const hits = [...props].filter((p) => PAINT_LAYOUT_PROPS.has(p));
    if (hits.length) {
      issues.push(`[animation]      ${rel} :: @keyframes ${name} (infinite) -> ${hits.join(', ')}`);
    }
  }
}

if (issues.length) {
  console.error(`✖ 渲染性能检查发现 ${issues.length} 处违规：\n`);
  for (const s of issues) console.error('  ' + s);
  console.error('\n规则见 docs/ui-glass-design.md 末尾「性能铁律」两节。');
  process.exit(1);
} else {
  console.log('✔ 渲染性能检查通过：无逐列表项 backdrop-filter / 无危险 infinite 动画 / 无滥用 will-change');
}
