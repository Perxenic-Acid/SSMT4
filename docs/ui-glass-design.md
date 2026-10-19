# Sci-Fi White Glass UI Design

> 科幻白透玻璃风格 — 半透明白色毛玻璃 + 浅蓝水晶点缀

## Design Philosophy

- **底色**：纯白半透明 `rgba(255,255,255,0.06)`，绝不用深蓝/深紫基底
- **毛玻璃**：强模糊 `blur(28px) saturate(1.6)`，透出背后内容
- **边框**：白色半透明 `rgba(255,255,255,0.12~0.20)`，有微弱光泽
- **点缀色**：浅蓝水晶 `#7DDCFF` / `rgba(125,220,255,...)`，用于徽章和 accent 线
- **阴影**：柔和 `rgba(0,0,0,0.30)` 大阴影，边缘清晰
- **文字**：纯白到浅灰渐变，无蓝色调

## Core Tokens

```css
/* Modal / Dialog background */
background: rgba(255,255,255,0.06);
backdrop-filter: blur(28px) saturate(1.6);
-webkit-backdrop-filter: blur(28px) saturate(1.6);

/* Border */
border: 1px solid rgba(255,255,255,0.20);
box-shadow:
  0 24px 64px rgba(0,0,0,0.30),
  0 0 0 1px rgba(255,255,255,0.08) inset;

/* Overlay backdrop */
background: rgba(0,0,0,0.25);
backdrop-filter: blur(4px);

/* Top hairline (:before) */
background: linear-gradient(90deg, transparent, rgba(255,255,255,0.15~0.18), transparent);

/* Crystal accent line */
background: rgba(125,220,255,0.20~0.60);
/* For left stripe on cards */
width: 3px; border-radius: 0 3px 3px 0;

/* Dot pattern overlay (:after) */
background-image: radial-gradient(circle, rgba(255,255,255,0.04) 1px, transparent 1px);
background-size: 20px 20px;
```

## Layout Constants

| Token | Value |
|---|---|
| Border radius (modal/dialog) | `24px` |
| Border radius (card) | `16px` |
| Border radius (input) | `8px` |
| Border radius (button) | `10~14px` |
| Border radius (badge) | `999px` |
| Section padding | `22px 28px 22px 32px` |
| Section divider | `::before` pseudo, `left:32px; right:28px; height:1px` |
| Crystal stripe on left | `width:3px; left:0; top:0; bottom:0` |
| Card / modal corner radius | `overflow:hidden` + `border-radius` |

## Typography

| Element | Size | Weight | Color |
|---|---|---|---|
| Modal title | `16px` | `700` | `rgba(255,255,255,0.90)` |
| Card title | `14~15px` | `700` | `rgba(255,255,255,0.88)` |
| Body / value | `12~13px` | `400` | `rgba(255,255,255,0.65)` |
| Meta / secondary | `11px` | `500` | `rgba(255,255,255,0.35~0.45)` |
| Label (uppercase) | `10px` | `700` | `rgba(255,255,255,0.40)` |
| Binding / code | `11px` | `500` | `rgba(255,255,255,0.45)` (monospace) |
| Condition text | `11px` | `400` | `rgba(125, 220, 255,0.60)` |
| Badge text | `9px` | `700` | `rgba(255,255,255,0.55)` |
| Button text | `12~13px` | `600~700` | `rgba(255,255,255,0.70~0.80)` |

## Component Patterns

### Modal / Dialog (top-level popup)

```
Overlay:   rgba(0,0,0,0.25) + blur(4px)
Container: rgba(255,255,255,0.06) + blur(28px) saturate(1.6)
Border:    1px solid rgba(255,255,255,0.20)
Shadow:    0 24px 64px rgba(0,0,0,0.30) + inset glow
Optional:  dot pattern ::after + white ambient glow ::before
```

### Item Card (inside modal)

```
Background: rgba(255,255,255,0.035~0.04)
Border:     1px solid rgba(255,255,255,0.10)
Shadow:     0 2px 8px rgba(0,0,0,0.06)
Top line:   ::before hairline, gradient white(0.18)
Left line:  ::after crystal stripe, 3px wide (0.20 → 0.60 on hover)
Hover:      background → rgba(255,255,255,0.06~0.07)
            border → 0.16~0.20
            shadow → 0 12px 32px rgba(0,0,0,0.12~0.15)
```

### Input

```
Background: rgba(255,255,255,0.04)
Border:     1px solid rgba(255,255,255,0.10)
Text color: rgba(255,255,255,0.85)
Padding:    9px 12px
Hover:      border → rgba(255,255,255,0.20)
Focus:      border → rgba(255,255,255,0.40)
Placeholder: rgba(255,255,255,0.30)
```

### Button / Action

```
Default:  rgba(255,255,255,0.05) + border(0.12)
Hover:    rgba(255,255,255,0.09~0.10) + border(0.28)
Primary:  white glass + crystal text on hover (#7DDCFF)
Danger:   rgba(255,70,70,0.12~0.15) on hover
Disabled: opacity(0.30) + grayscale(0.5)
```

### Badge / Chip

```
Background: rgba(255,255,255,0.06)
Border:     1px solid rgba(255,255,255,0.15)
Color:      rgba(255,255,255,0.55)
Or crystal: rgba(125,220,255,0.08~0.12) bg
            rgba(125,220,255,0.12~0.20) border
            rgba(125,220,255,0.60~0.75) text
```

### Section Divider (inside card)

Use `::before` pseudo-element, NOT `border-top`:

```css
.ek-card-section + .ek-card-section::before {
  content: '';
  position: absolute;
  top: 0;
  left: 32px;     /* match section padding-left */
  right: 28px;    /* match section padding-right */
  height: 1px;
  background: rgba(255,255,255,0.06);
  pointer-events: none;
}
```

This avoids the divider line bleeding into the card's `border-radius` corners.

## What NOT To Do

- ❌ 不要使用深蓝/紫色基底 (`rgba(28,32,48,0.72)`)
- ❌ 不要使用金色/橙色 accent 线（浅蓝水晶 only）
- ❌ 不要使用暖色光晕（只用白色环境光 + 浅蓝水晶辉光）
- ❌ 不要在 section 上用 `border-top` 分隔（改用 ::before pseudo）
- ❌ 不要用饱和度过高的颜色 — 所有颜色都是低饱和度、高透明度
- ❌ 不要用 `el-dialog`、`el-button`、`el-input` 的默认样式 — 全部覆盖

## Example: Home page start-game-btn

```css
.start-game-btn {
  background: rgba(255,255,255,0.08);
  border: 1px solid rgba(255,255,255,0.12);
  border-radius: 30px;
  box-shadow: 0 4px 24px rgba(0,0,0,0.08);
  color: rgba(255,255,255,0.9);
}
.start-game-btn:hover {
  background: rgba(255,255,255,0.12);
  border-color: rgba(255,255,255,0.2);
}
```

## Quick Copy-Paste Template

```css
/* White glass container */
.my-panel {
  background: rgba(255,255,255,0.06);
  border: 1px solid rgba(255,255,255,0.20);
  border-radius: 24px;
  backdrop-filter: blur(28px) saturate(1.6);
  -webkit-backdrop-filter: blur(28px) saturate(1.6);
  box-shadow: 0 24px 64px rgba(0,0,0,0.30), 0 0 0 1px rgba(255,255,255,0.08) inset;
  overflow: hidden;
}
/* Top hairline */
.my-panel::before {
  content: ''; position: absolute;
  top: 0; left: 0; right: 0; height: 1px;
  background: linear-gradient(90deg, transparent, rgba(255,255,255,0.15), transparent);
  pointer-events: none;
}
/* Dot pattern */
.my-panel::after {
  content: ''; position: absolute; inset: 0;
  pointer-events: none; z-index: 0;
  background-image: radial-gradient(circle, rgba(255,255,255,0.04) 1px, transparent 1px);
  background-size: 20px 20px;
}
```

相关文件：`SwitchKeyList.vue`、`EditSwitchKeyList.vue`、`Home.vue`（参考）

## ⚠️ 性能铁律：backdrop-filter 的使用边界

`backdrop-filter` 是 Chromium（WebView2）中代价最高的合成效果：每个实例都要
"截取背景 → GPU 模糊 → 合成"，滚动时还需对可见区域重新光栅化。当模糊层数量随
列表规模增长时，会触发合成器 bug —— **滚动时背景闪烁、文字消失**（本仓库
WorkPage / ModsManagement 均实际出现过，已修复）。

**只允许**在以下元素上使用 `backdrop-filter`：

- 数量固定、尺寸有界的容器：页面级 `.panel` / `.glass-panel`、侧边栏、工具栏
- 瞬时浮层：dialog / overlay / popover / tooltip / dropdown / context-menu / lightbox

**禁止**在以下元素上使用 `backdrop-filter`：

- `v-for` 列表项及其内部任何子元素（卡片、徽章、按钮、输入框、标签）
- 表格（`el-table`）整体、行、单元格
- 画布/蓝图节点等数量不定、可拖动、可平移的元素
- 高度随内容增长的可滚动容器内部的嵌套模糊层（模糊由外层固定面板统一提供）

列表项需要"玻璃感"时，只用半透明 `rgba(...)` 背景 + 边框即可 —— 它们的背景
通常已经是被外层面板模糊过的内容，再叠一层模糊视觉上几乎无差别，但每层都在
烧合成器预算。

## ⚠️ 动画性能：只动画合成器属性

无限循环的装饰动画**只能**动画 `transform` / `opacity`（合成器属性，GPU 混合，
零重绘）。禁止动画以下属性，尤其是在列表项上：

- `box-shadow` / `border-color` / `background` / `background-position` —— 每帧主线程重绘
- `left` / `top` / `width` / `height` / `margin` —— 每帧触发布局 + 重绘

等效改造手法：

- 阴影/边框"呼吸"：把峰值状态放到伪元素上，动画其 `opacity`（见
  `ModCard.vue` 的 `.mod-card:not(.is-disabled)::before`）
- 扫光位移：`left` 位移换算为等价的 `translateX`（注意 translate 百分比相对
  自身宽度，left 百分比相对父级宽度，需按比例换算，见 `subgroupSheen`）

`will-change` 只声明确实会动画的属性；`will-change: box-shadow/filter` 会让
每个元素常驻多余提升层，浪费显存。

**回归门禁**：提交前运行 `npm run check:render-perf`
（`scripts/check-render-perf.mjs`），它会自动拦截以上三类违规。
确有界场景需要豁免时，在脚本的 `ACCEPTED` 白名单中登记并注明理由。

