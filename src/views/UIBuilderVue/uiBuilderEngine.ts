// @ts-nocheck
/* ═══════════════════════════════════════════════════════════════
   UI 构造器引擎 — 由 public/ui-builder-v79.html 内嵌脚本机械移植而来。

   - 原引擎为单文件 iframe 页面;移植后由 UIBuilderApp.vue 在挂载时
     调用 createUIBuilderEngine(root) 注入,内部 DOM 绑定以挂载后的
     根节点为准。
   - 原页面内联 onclick/onchange 等处理器依赖全局作用域;移植后统一
     挂到 window.UIB 命名空间(事件属性已做前缀转换)。
   - postMessage 宿主桥替换为 hostBridge.ts 直连工作空间保存。
   - alert/confirm/prompt 由 dialogs.ts 全局替换为自绘制弹窗
     (confirm/prompt 调用点已改为异步)。
   - 新增 __onWindow/__onDocument 注册表,供 destroy() 清理。
   ═══════════════════════════════════════════════════════════════ */
import { createDirectSsmHostBridge } from './hostBridge'

export function createUIBuilderEngine(root: HTMLElement) {
  const __windowListeners: Array<[string, EventListenerOrEventListenerObject, boolean | AddEventListenerOptions | undefined]> = []
  const __documentListeners: Array<[string, EventListenerOrEventListenerObject, boolean | AddEventListenerOptions | undefined]> = []
  const __onWindow = (type: string, fn: EventListenerOrEventListenerObject, opts?: boolean | AddEventListenerOptions) => {
    window.addEventListener(type, fn, opts)
    __windowListeners.push([type, fn, opts])
  }
  const __onDocument = (type: string, fn: EventListenerOrEventListenerObject, opts?: boolean | AddEventListenerOptions) => {
    document.addEventListener(type, fn, opts)
    __documentListeners.push([type, fn, opts])
  }

    function updateUIScale(val) {
        root.style.setProperty('--ui-scale', val);
        document.getElementById('ui_scale_val').innerText = Math.round(val * 100) + '%';
    }

    let components = [];
    let groups = [];
    let roots = [];
    let selectedEntities = [];
    let selectedEntity = null;
    let selectedIds = [];
    let selectedId = null;
    let dragSession = null;
    let isDraggingCanvasEntity = false;
    let previewClockHandle = null;
    const HISTORY_LIMIT = 200;
    let historyBaseState = null;
    let historyPatches = [];
    let historyIndex = 0;
    let historyCurrentState = null;
    let historyCurrentSerialized = '';
    let isRestoringHistory = false;
    let historyDirty = false;

    const workArea = document.getElementById('workArea');
    const propPanel = document.getElementById('propPanel');
    const animPanel = document.getElementById('animPanel');
    const animColumn = document.getElementById('animColumn');
    const componentPanelWrap = document.getElementById('componentPanelWrap');
    const hierarchyPanel = document.getElementById('hierarchy_panel');
    const varContainer = document.getElementById('var_container');
    const modeSelect = document.getElementById('p_mode');
    const resourceWindow = document.getElementById('resourceWindow');
    const resourceWindowTitle = document.getElementById('resourceWindowTitle');
    const resourceScopeLabel = document.getElementById('resourceScopeLabel');
    const resContainer = document.getElementById('res_container');
    let hierarchyRenderCache = '';
    const hierarchyCollapsedGroups = new Set();
    let hierarchySelectAnchor = null;
    let hierarchyDragRef = null;
    let dialogueLogic = createDefaultDialogueLogic();
    let workspaceMode = 'layout';
    let blueprintScopeId = 'main';
    let selectedBlueprintNodeIds = [];
    let blueprintPan = { x: 0, y: 0, scale: 1 };
    let blueprintConnectionDraft = null;
    let blueprintSuppressContextMenu = false;
    let dialogueRuntime = null;
    let dialogueRuntimeFrame = null;

    function createDefaultDialogueLogic() {
        return {
            version: 1,
            variables: [],
            main: { nodes: [], edges: [], view: { x: 0, y: 0, scale: 1 } },
            dialogues: []
        };
    }

    function normalizeDialogueLogic(value) {
        const source = value && typeof value === 'object' ? cloneDeep(value) : createDefaultDialogueLogic();
        source.version = 1;
        source.variables = Array.isArray(source.variables) ? source.variables : [];
        source.main = source.main && typeof source.main === 'object' ? source.main : { nodes: [], edges: [], view: {} };
        source.main.nodes = Array.isArray(source.main.nodes) ? source.main.nodes : [];
        source.main.edges = Array.isArray(source.main.edges) ? source.main.edges : [];
        source.main.view = normalizeBlueprintView(source.main.view);
        source.dialogues = Array.isArray(source.dialogues) ? source.dialogues : [];
        source.dialogues.forEach(dialogue => {
            dialogue.id = dialogue.id || `dialogue_${nextUniqueToken()}`;
            dialogue.groupId = String(dialogue.groupId || '');
            dialogue.name = String(dialogue.name || '对话编组');
            dialogue.entryNodeId = String(dialogue.entryNodeId || '');
            dialogue.nodes = Array.isArray(dialogue.nodes) ? dialogue.nodes : [];
            dialogue.edges = Array.isArray(dialogue.edges) ? dialogue.edges : [];
            dialogue.view = normalizeBlueprintView(dialogue.view);
        });
        source.variables = source.variables.map(item => ({
            name: sanitizeIniVarToken(item && item.name, ''),
            initialValue: Number.isFinite(Number(item && item.initialValue)) ? Number(item.initialValue) : 0
        })).filter(item => item.name);
        return source;
    }

    function normalizeBlueprintView(view) {
        return {
            x: Number.isFinite(Number(view && view.x)) ? Number(view.x) : 0,
            y: Number.isFinite(Number(view && view.y)) ? Number(view.y) : 0,
            scale: Math.max(0.35, Math.min(2.5, Number(view && view.scale) || 1))
        };
    }

    function getBlueprintGraph(scopeId = blueprintScopeId) {
        if(scopeId === 'main') return dialogueLogic.main;
        return dialogueLogic.dialogues.find(item => item.id === scopeId) || dialogueLogic.main;
    }

    function getDialogueDefinitionForGroup(groupId) {
        return dialogueLogic.dialogues.find(item => item.groupId === groupId) || null;
    }

    function getOrCreateDialogueForGroup(groupId) {
        let dialogue = getDialogueDefinitionForGroup(groupId);
        if(dialogue) return dialogue;
        const group = getGroupById(groupId);
        dialogue = {
            id: `dialogue_${nextUniqueToken()}`,
            groupId,
            name: `${group ? sanitizeGroupDisplayName(group.name, group.id) : '编组'} 对话`,
            entryNodeId: '', nodes: [], edges: [], view: { x: 0, y: 0, scale: 1 }
        };
        dialogueLogic.dialogues.push(dialogue);
        return dialogue;
    }

    function getComponentDialogueMembership(componentId) {
        for(const dialogue of dialogueLogic.dialogues) {
            for(const node of dialogue.nodes || []) {
                if(node.type === 'step' && Array.isArray(node.config && node.config.textIds) && node.config.textIds.includes(componentId)) {
                    return { dialogue, step: node };
                }
            }
        }
        return null;
    }

    function collectDialogueVariableNames() {
        const names = new Set();
        const visitNode = node => {
            const cfg = node && node.config || {};
            (cfg.clauses || []).forEach(clause => {
                const name = sanitizeIniVarToken(clause && clause.variable, '');
                if(name) names.add(name);
            });
            (cfg.assignments || []).forEach(action => {
                const name = sanitizeIniVarToken(action && action.variable, '');
                if(name) names.add(name);
            });
        };
        (dialogueLogic.main.nodes || []).forEach(visitNode);
        dialogueLogic.dialogues.forEach(dialogue => (dialogue.nodes || []).forEach(visitNode));
        names.forEach(name => {
            if(!dialogueLogic.variables.some(item => item.name === name)) dialogueLogic.variables.push({ name, initialValue: 0 });
        });
        dialogueLogic.variables = dialogueLogic.variables.filter(item => names.has(item.name));
        return names;
    }

    const DEFAULT_HS = 0.022;
    const DEFAULT_TT = 0.006;
    const BASE_HEIGHT = 720;
    const EDGE_DOCK_TOUCH_EPSILON = 0.006;
    const PREVIEW_BG_MIN_OPACITY = 0.9;
    const RUNTIME_BG_MIN_ALPHA = 0.9;
    const DEFAULT_FLOW_SPEED_SCALE = 0.1;
    const PERSISTENT_ANIM_DEFAULTS = Object.freeze({
        enabled: true,
        speedMultiplier: 0.03,
        flowSpeedScale: DEFAULT_FLOW_SPEED_SCALE
    });
    const SHORTCUT_DEFINITIONS = Object.freeze([
        { name: 'help', inputId: 'shortcut_help', defaultValue: 'no_ctrl no_alt home', label: '面板开关' },
        { name: 'layout', inputId: 'shortcut_layout', defaultValue: 'ctrl e', label: '布局模式' },
        { name: 'reset', inputId: 'shortcut_reset', defaultValue: 'ctrl home', label: '重置位置' },
        { name: 'zoomIn', inputId: 'shortcut_zoom_in', defaultValue: 'up', label: '放大' },
        { name: 'zoomOut', inputId: 'shortcut_zoom_out', defaultValue: 'down', label: '缩小' },
        { name: 'dockModifier', inputId: 'shortcut_dock', defaultValue: 'alt', label: '停靠修饰键' },
        { name: 'drag', inputId: 'shortcut_drag', defaultValue: 'no_ctrl alt Q', label: '备用拖拽' },
        { name: 'mouseDrag', inputId: 'shortcut_mouse', defaultValue: 'VK_LBUTTON', label: '鼠标拖拽' }
    ]);
    const JOYSTICK_LEGACY_DIR_REMAP = [0, 3, 1, 2];
    const AUTO_FUNCTION_SAMPLE_COUNT = 64;
    const GLOBAL_ANIM_MODE_OPTIONS = Object.freeze([
        { value: 'none', label: '无', hint: '不对组件或编组追加额外整体动画。' },
        { value: 'edge_dock', label: '靠边收纳', hint: '组件或编组会向屏幕边缘缩回，鼠标靠近触发区域后再平滑展开。适合菜单面板、侧栏、工具条。' },
        { value: 'group_float_y', label: '整体浮动', hint: '整个组件或编组沿纵向轻微漂浮，适合悬浮卡片、状态面板。' },
        { value: 'group_float_x', label: '整体横移', hint: '整个组件或编组沿横向轻微摆动，适合呼吸式工具条或提醒条。' },
        { value: 'group_pulse', label: '整体呼吸', hint: '整个组件或编组进行轻微缩放脉冲，适合强调组、确认条。' }
    ]);
    const LOCAL_ANIM_LIBRARY = Object.freeze({
        default: [
            { value: 'none', label: '无', hint: '只保留静态显示。' },
            { value: 'breathe', label: '呼吸起伏', hint: '做轻微缩放和亮度呼吸，适合大多数普通面板。' },
            { value: 'shimmer', label: '流光扫过', hint: '让表面高光周期性扫过，适合卡片、按钮和装饰板。' }
        ],
        slider_h: [
            { value: 'none', label: '无', hint: '静态显示。' },
            { value: 'handle_bob', label: '滑块弹跳', hint: '滑块沿轨道中心做小幅起伏，强调可拖拽感。' },
            { value: 'fill_breathe', label: '填充呼吸', hint: '已填充区域轻微明暗脉冲，适合能量条、进度条。' },
            { value: 'sheen', label: '轨道流光', hint: '沿横向做扫光，适合高光金属或玻璃滑条。' }
        ],
        slider_v: [
            { value: 'none', label: '无', hint: '静态显示。' },
            { value: 'handle_bob', label: '滑块弹跳', hint: '滑块沿轨道中心做小幅起伏，强调可拖拽感。' },
            { value: 'fill_breathe', label: '填充呼吸', hint: '已填充区域轻微明暗脉冲，适合竖直能量条。' },
            { value: 'sheen', label: '轨道流光', hint: '沿纵向做扫光，适合科技仪表。' }
        ],
        accum: [
            { value: 'none', label: '无', hint: '静态显示。' },
            { value: 'fill_breathe', label: '填充呼吸', hint: '已积蓄区域轻微明暗脉冲，适合能量条。' },
            { value: 'sheen', label: '轨道流光', hint: '沿横向做扫光，适合高光积蓄条。' }
        ],
        joystick: [
            { value: 'none', label: '无', hint: '静态显示。' },
            { value: 'gyro_orbit', label: '陀螺环动', hint: '围绕摇杆中心做轻微旋摆，适合方向控制盘。' },
            { value: 'handle_breathe', label: '摇杆心跳', hint: '摇杆头轻微缩放呼吸，突出可操作焦点。' },
            { value: 'radial_sheen', label: '径向流光', hint: '圆周高光沿边缘环扫，适合科幻摇杆。' }
        ],
        toggle: [
            { value: 'none', label: '无', hint: '静态切换。' },
            { value: 'toggle_slide', label: '开关滑移', hint: '通过内部插值模拟切换时的过渡位移和亮度变化。' },
            { value: 'toggle_pop', label: '开关弹出', hint: '切换态做轻微放大回弹，适合拟物开关。' },
            { value: 'sheen', label: '表面流光', hint: '给开关外壳或亮起段增加扫光。' }
        ],
        static: [
            { value: 'none', label: '无', hint: '静态显示。' },
            { value: 'breathe', label: '呼吸起伏', hint: '适合信息面板、图标底板。' },
            { value: 'shimmer', label: '流光扫过', hint: '适合卡片、玻璃板。' },
            { value: 'tilt', label: '轻微倾摆', hint: '适合装饰节点和浮层。' }
        ],
        sequence: [
            { value: 'none', label: '无', hint: '只使用序列帧切换。' },
            { value: 'breathe', label: '呼吸起伏', hint: '在序列帧外层增加整体脉动。' },
            { value: 'sheen', label: '流光扫过', hint: '对序列画面外层加扫光。' }
        ],
        text: [
            { value: 'none', label: '无', hint: '静态文本显示。' },
            { value: 'glyph_wave', label: '字符波动', hint: '文字整体做细微起伏，适合提示词、标题。' },
            { value: 'glyph_glow', label: '字符辉光', hint: '文字做周期性发光，适合状态提示。' },
            { value: 'type_breathe', label: '文本呼吸', hint: '文本块轻微缩放和透明度起伏。' }
        ]
    });
    const GLOBAL_ANIM_DEFAULTS = Object.freeze({
        mode: 'none',
        edge: 'auto',
        strength: 0.1,
        speed: 0.02,
        reveal: 0.05,
        trigger: 0.04,
        ease: 0.16
    });
    const LOCAL_ANIM_DEFAULTS = Object.freeze({
        mode: 'none',
        strength: 0.18,
        speed: 0.002
    });
    const LOCAL_ANIM_RECOMMENDED = Object.freeze({
        slider_h: 'fill_breathe',
        slider_v: 'fill_breathe',
        joystick: 'handle_breathe',
        toggle: 'toggle_pop',
        accum: 'fill_breathe',
        static: 'shimmer',
        sequence: 'breathe',
        text: 'glyph_glow'
    });
    const GLOBAL_ANIM_RUNTIME_MODE = Object.freeze({
        none: 0,
        edge_dock: 1,
        group_float_y: 2,
        group_float_x: 3,
        group_pulse: 4
    });
    const LOCAL_ANIM_RUNTIME_MODE = Object.freeze({
        none: 0,
        breathe: 1,
        shimmer: 2,
        handle_bob: 3,
        fill_breathe: 4,
        gyro_orbit: 5,
        handle_breathe: 6,
        radial_sheen: 7,
        toggle_slide: 8,
        toggle_pop: 9,
        glyph_wave: 10,
        glyph_glow: 11,
        type_breathe: 12,
        sheen: 13,
        tilt: 14
    });
    const ROOT_ENTITY_ID = '__root__';
    const AUTO_FUNCTION_ALLOWED_IDENTIFIERS = new Set([
        't', 'PI', 'TAU', 'E',
        'sin', 'cos', 'tan', 'abs', 'min', 'max', 'pow', 'sqrt', 'log', 'exp',
        'floor', 'ceil', 'round', 'sign', 'clamp', 'fract', 'mix', 'step',
        'smoothstep', 'saw', 'tri', 'pingpong', 'sin01', 'cos01'
    ]);
    const DEFAULT_SHADER_SOURCE = String.raw`// **** RESPONSIVE UI SHADER WITH MOBILE-STYLE SURFACE ENHANCEMENT ****
// Updated for v75

Texture1D<float4> IniParams : register(t120);

// x87, y87 = Width, Height (0..1 screen space)
// z87, w87 = Top-Left X, Top-Left Y (0..1 screen space)
// x86      = Rotation in Radians
// y86      = Aspect Ratio (Width / Height)
// z86      = Roundness (>=0 legacy ratio, <0 fixed corner radius in design-height units)
// x90      = Alpha multiplier
// y90      = Animation phase
// z90      = Extra rotation
// w90      = Effect boost

#define SIZE IniParams[87].xy
#define TL_POS IniParams[87].zw
#define ROTATION IniParams[86].x
#define ASPECT IniParams[86].y
#define ROUNDNESS IniParams[86].z
#define ALPHA_MUL IniParams[90].x
#define ANIM_PHASE IniParams[90].y
#define EXTRA_ROT IniParams[90].z
#define EFFECT_BOOST IniParams[90].w
#define SURFACE_FX_ENABLED IniParams[91].x

struct vs2ps {
    float4 pos : SV_Position0;
    float2 uv : TEXCOORD1;
};

#ifdef VERTEX_SHADER
void main(
        out vs2ps output,
        uint vertex : SV_VertexID)
{
    float2 centerUV = TL_POS.xy + SIZE.xy * 0.5;
    float2 hSize = SIZE.xy * 0.5;
    float2 localPos;
    float2 uv;

    switch(vertex) {
        case 0: localPos = float2(hSize.x, -hSize.y); uv = float2(1, 0); break;
        case 1: localPos = float2(hSize.x,  hSize.y); uv = float2(1, 1); break;
        case 2: localPos = float2(-hSize.x, -hSize.y); uv = float2(0, 0); break;
        case 3: localPos = float2(-hSize.x,  hSize.y); uv = float2(0, 1); break;
        default: localPos = float2(0, 0); uv = float2(0, 0); break;
    };

    localPos.x *= ASPECT;

    float c = cos(ROTATION + EXTRA_ROT);
    float s = sin(ROTATION + EXTRA_ROT);
    float2 rotatedPos;
    rotatedPos.x = localPos.x * c - localPos.y * s;
    rotatedPos.y = localPos.x * s + localPos.y * c;
    rotatedPos.x /= ASPECT;

    float2 finalUV = centerUV + rotatedPos;
    output.pos.x = finalUV.x * 2.0 - 1.0;
    output.pos.y = (1.0 - finalUV.y) * 2.0 - 1.0;
    output.pos.zw = float2(0, 1);
    output.uv = uv;
}
#endif

#ifdef PIXEL_SHADER
Texture2D<float4> tex : register(t100);

float luminance(float3 c)
{
    return dot(c, float3(0.2126, 0.7152, 0.0722));
}

float resolveCornerRadius(float roundness, float2 halfSize)
{
    float minSide = min(halfSize.x, halfSize.y);
    if (roundness < 0.0) return clamp(-roundness, 0.0, minSide);
    return saturate(roundness) * minSide;
}

float shapeDistance(float2 uv, float roundness)
{
    float2 quadDims = max(float2(SIZE.x * ASPECT, SIZE.y), float2(0.0001, 0.0001));
    float2 halfSize = quadDims * 0.5;
    float radius = resolveCornerRadius(roundness, halfSize);
    float2 p = (uv * 2.0 - 1.0) * halfSize;
    float2 q = abs(p) - (halfSize - radius);
    float outside = length(max(q, 0.0));
    float inside = min(max(q.x, q.y), 0.0);
    return radius - (outside + inside);
}

float roundedShapeMask(float2 uv, float roundness)
{
    float2 quadDims = max(float2(SIZE.x * ASPECT, SIZE.y), float2(0.0001, 0.0001));
    float feather = max(min(quadDims.x, quadDims.y) * 0.01, 0.0005);
    return smoothstep(0.0, feather, shapeDistance(uv, roundness));
}
void main(vs2ps input, out float4 result : SV_Target0)
{
    float2 dims;
    tex.GetDimensions(dims.x, dims.y);
    if (!dims.x || !dims.y) discard;

    float2 sampleUV = saturate(input.uv);
    float4 base = tex.Load(int3(sampleUV.xy * dims.xy, 0));
    if (base.a <= 0.001) discard;
    float shapeMask = roundedShapeMask(sampleUV, ROUNDNESS);
    if (shapeMask <= 0.001) discard;

    float2 centered = sampleUV * 2.0 - 1.0;
    float radialDist = length(centered);
    float edgeDist = max(abs(centered.x), abs(centered.y));
    float edgeMask = smoothstep(0.58, 0.98, edgeDist);
    float borderMask = smoothstep(0.82, 1.0, edgeDist);

    float topSpec = pow(saturate(1.0 - sampleUV.y), 2.7);
    float sweepCoord = frac(sampleUV.x * 0.76 + sampleUV.y * 0.58 - ANIM_PHASE);
    float sheenBand = pow(saturate(1.0 - abs(sweepCoord - 0.5) / 0.22), 2.0);
    float diagBand = smoothstep(0.16, 0.42, sampleUV.x + sampleUV.y) * (1.0 - smoothstep(0.52, 0.86, sampleUV.x + sampleUV.y));
    float cornerLift = pow(saturate(1.0 - distance(sampleUV, float2(0.18, 0.14)) * 1.45), 2.0);
    float radialBand = pow(saturate(1.0 - abs(radialDist - 0.72) / 0.22), 2.3);
    float boost = saturate(EFFECT_BOOST);

    float3 accentCool = float3(0.48, 0.82, 1.0);
    float3 accentWarm = float3(1.0, 0.84, 0.56);
    float3 edgeTint = lerp(accentCool, accentWarm, saturate(luminance(base.rgb) * 1.15));
    float3 color = base.rgb;
    if (SURFACE_FX_ENABLED > 0.5) {
        color += accentCool * topSpec * 0.16;
        color += float3(1.0, 1.0, 1.0) * sheenBand * (0.05 + boost * 0.16);
        color += edgeTint * diagBand * 0.12;
        color += accentWarm * cornerLift * 0.10;
        color += edgeTint * edgeMask * (0.08 + boost * 0.18);
        color += accentCool * radialBand * boost * 0.18;
        color = lerp(color, color * (1.08 + boost * 0.12), borderMask * (0.16 + boost * 0.12));
    }
    result = float4(saturate(color), base.a * shapeMask * max(ALPHA_MUL, 0.0));
}
#endif
`;

    function cloneDeep(value) {
        return JSON.parse(JSON.stringify(value));
    }

    function getInputValue(id, fallback = '') {
        const input = document.getElementById(id);
        return input ? input.value : fallback;
    }

    function setInputValue(id, value) {
        const input = document.getElementById(id);
        if(input) input.value = value;
    }

    function getInputChecked(id, fallback = false) {
        const input = document.getElementById(id);
        return input ? input.checked : fallback;
    }

    function setInputChecked(id, value) {
        const input = document.getElementById(id);
        if(input) input.checked = !!value;
    }

    function normalizeShortcutValue(value) {
        return String(value == null ? '' : value).replace(/\+/g, ' ').trim().replace(/\s+/g, ' ');
    }

    function inspectShortcutSettings() {
        const settings = {};
        const errors = [];
        const owners = new Map();
        SHORTCUT_DEFINITIONS.forEach(def => {
            const input = document.getElementById(def.inputId);
            const value = normalizeShortcutValue(input ? input.value : def.defaultValue);
            settings[def.name] = value;
            let error = '';
            if(!value) {
                error = `${def.label}不能为空`;
            } else if(!/^[A-Za-z0-9_ ]+$/.test(value)) {
                error = `${def.label}含有无效字符`;
            } else {
                const signature = value.toLowerCase();
                if(owners.has(signature)) {
                    error = `${def.label}与${owners.get(signature)}重复`;
                } else {
                    owners.set(signature, def.label);
                }
            }
            if(input) input.setAttribute('aria-invalid', error ? 'true' : 'false');
            if(error) errors.push(error);
        });
        return { settings, errors };
    }

    function validateShortcutSettings() {
        const result = inspectShortcutSettings();
        const status = document.getElementById('shortcut_status');
        if(status) {
            status.textContent = result.errors.length > 0 ? result.errors.join('；') : '快捷键设置有效';
            status.classList.toggle('error', result.errors.length > 0);
        }
        return result.errors.length === 0;
    }

    function getShortcutSettings() {
        const result = inspectShortcutSettings();
        validateShortcutSettings();
        if(result.errors.length > 0) throw new Error(`快捷键设置无效：${result.errors.join('；')}`);
        return result.settings;
    }

    function applyShortcutSettings(settings = {}) {
        SHORTCUT_DEFINITIONS.forEach(def => {
            setInputValue(def.inputId, normalizeShortcutValue(settings[def.name] || def.defaultValue));
        });
        validateShortcutSettings();
    }

    function normalizeShortcutInput(input) {
        if(input) input.value = normalizeShortcutValue(input.value);
        validateShortcutSettings();
    }

    function resetShortcutSettings() {
        markHistoryDirty();
        applyShortcutSettings({});
    }

    function getEditorControlState() {
        return {
            hash: getInputValue('char_hash', 'c209c22b'),
            matchIndex: getInputValue('match_index', ''),
            matchFirstIndex: getInputValue('match_first_index', ''),
            aspect: getInputValue('global_aspect', 1.777),
            gridSnapX: getInputValue('grid_snap_x', 0.02),
            gridSnapY: getInputValue('grid_snap_y', 0.03554),
            gridSnapAutoY: getInputChecked('grid_snap_auto_y', true),
            shortcuts: inspectShortcutSettings().settings,
            persistentAnimEnabled: getInputChecked('p_anim_persistent_enabled', PERSISTENT_ANIM_DEFAULTS.enabled),
            persistentAnimSpeed: getInputValue('p_anim_persistent_speed', PERSISTENT_ANIM_DEFAULTS.speedMultiplier),
            persistentAnimFlowSpeed: getInputValue('p_anim_persistent_flow_speed', PERSISTENT_ANIM_DEFAULTS.flowSpeedScale)
        };
    }

    function applyEditorControlState(controls = {}) {
        setInputValue('char_hash', controls.hash || 'c209c22b');
        setInputValue('match_index', controls.matchIndex || '');
        setInputValue('match_first_index', controls.matchFirstIndex || '');
        setInputValue('global_aspect', controls.aspect || 1.777);
        setInputValue('grid_snap_x', controls.gridSnapX || 0.02);
        setInputValue('grid_snap_y', controls.gridSnapY || 0.03554);
        setInputChecked('grid_snap_auto_y', controls.gridSnapAutoY !== false);
        applyShortcutSettings(controls.shortcuts || {});
        setInputChecked('p_anim_persistent_enabled', controls.persistentAnimEnabled !== false);
        setInputValue('p_anim_persistent_speed', controls.persistentAnimSpeed ?? PERSISTENT_ANIM_DEFAULTS.speedMultiplier);
        setInputValue('p_anim_persistent_flow_speed', controls.persistentAnimFlowSpeed ?? PERSISTENT_ANIM_DEFAULTS.flowSpeedScale);
    }

    function getSelectionSnapshot() {
        return {
            selectedEntities: cloneDeep(selectedEntities),
            selectedEntity: selectedEntity ? cloneDeep(selectedEntity) : null
        };
    }

    function captureEditorState() {
        return {
            components: cloneDeep(components),
            groups: cloneDeep(groups),
            roots: cloneDeep(roots),
            dialogueLogic: cloneDeep(dialogueLogic),
            selection: getSelectionSnapshot(),
            controls: getEditorControlState(),
            uniqueTokenCounter
        };
    }

    function serializeHistoryContent(state) {
        return JSON.stringify({
            components: state.components,
            groups: state.groups,
            roots: state.roots,
            dialogueLogic: state.dialogueLogic,
            controls: state.controls,
            uniqueTokenCounter: state.uniqueTokenCounter
        });
    }

    function serializeStable(value) {
        return JSON.stringify(value == null ? null : value);
    }

    function hashStableString(value) {
        const text = typeof value === 'string' ? value : serializeStable(value);
        let hash = 2166136261 >>> 0;
        for(let i = 0; i < text.length; i++) {
            hash ^= text.charCodeAt(i);
            hash = Math.imul(hash, 16777619) >>> 0;
        }
        return hash >>> 0;
    }

    function listToByIdMap(list) {
        const map = new Map();
        (list || []).forEach(item => {
            if(item && item.id) map.set(item.id, item);
        });
        return map;
    }

    function diffEntityListById(prevList, nextList) {
        const prevMap = listToByIdMap(prevList);
        const nextMap = listToByIdMap(nextList);
        const ids = new Set([...prevMap.keys(), ...nextMap.keys()]);
        const changed = [];
        ids.forEach(id => {
            const before = prevMap.has(id) ? prevMap.get(id) : null;
            const after = nextMap.has(id) ? nextMap.get(id) : null;
            if(serializeStable(before) !== serializeStable(after)) {
                changed.push({
                    id,
                    before: before ? cloneDeep(before) : null,
                    after: after ? cloneDeep(after) : null
                });
            }
        });
        return changed;
    }

    function buildHistoryPatch(beforeState, afterState, reason = '') {
        const beforeComponentOrder = (beforeState.components || []).map(item => item.id);
        const afterComponentOrder = (afterState.components || []).map(item => item.id);
        const beforeGroupOrder = (beforeState.groups || []).map(item => item.id);
        const afterGroupOrder = (afterState.groups || []).map(item => item.id);
        return {
            reason,
            components: diffEntityListById(beforeState.components, afterState.components),
            groups: diffEntityListById(beforeState.groups, afterState.groups),
            componentOrder: serializeStable(beforeComponentOrder) === serializeStable(afterComponentOrder)
                ? null
                : { before: beforeComponentOrder, after: afterComponentOrder },
            groupOrder: serializeStable(beforeGroupOrder) === serializeStable(afterGroupOrder)
                ? null
                : { before: beforeGroupOrder, after: afterGroupOrder },
            roots: serializeStable(beforeState.roots) === serializeStable(afterState.roots)
                ? null
                : { before: cloneDeep(beforeState.roots), after: cloneDeep(afterState.roots) },
            dialogueLogic: serializeStable(beforeState.dialogueLogic) === serializeStable(afterState.dialogueLogic)
                ? null
                : { before: cloneDeep(beforeState.dialogueLogic), after: cloneDeep(afterState.dialogueLogic) },
            controls: serializeStable(beforeState.controls) === serializeStable(afterState.controls)
                ? null
                : { before: cloneDeep(beforeState.controls), after: cloneDeep(afterState.controls) },
            uniqueTokenCounter: beforeState.uniqueTokenCounter === afterState.uniqueTokenCounter
                ? null
                : { before: beforeState.uniqueTokenCounter, after: afterState.uniqueTokenCounter },
            selection: {
                before: cloneDeep(beforeState.selection || {}),
                after: cloneDeep(afterState.selection || {})
            }
        };
    }

    function historyPatchHasContent(patch) {
        return !!(patch &&
            ((patch.components && patch.components.length > 0) ||
                (patch.groups && patch.groups.length > 0) ||
                patch.componentOrder ||
                patch.groupOrder ||
                patch.roots ||
                patch.dialogueLogic ||
                patch.controls ||
                patch.uniqueTokenCounter));
    }

    function setEntityListItem(list, id, value) {
        const nextList = (list || []).slice();
        const index = nextList.findIndex(item => item && item.id === id);
        if(!value) {
            return index >= 0 ? nextList.filter(item => item && item.id !== id) : nextList;
        }
        if(index >= 0) nextList[index] = cloneDeep(value);
        else nextList.push(cloneDeep(value));
        return nextList;
    }

    function reorderEntityList(list, order) {
        if(!Array.isArray(order)) return list || [];
        const byId = listToByIdMap(list);
        const used = new Set();
        const ordered = [];
        order.forEach(id => {
            if(byId.has(id)) {
                ordered.push(byId.get(id));
                used.add(id);
            }
        });
        (list || []).forEach(item => {
            if(item && item.id && !used.has(item.id)) ordered.push(item);
        });
        return ordered;
    }

    function applyHistoryPatchToState(state, patch, direction = 'after') {
        const next = cloneDeep(state);
        (patch.components || []).forEach(change => {
            next.components = setEntityListItem(next.components, change.id, change[direction]);
        });
        (patch.groups || []).forEach(change => {
            next.groups = setEntityListItem(next.groups, change.id, change[direction]);
        });
        if(patch.componentOrder) next.components = reorderEntityList(next.components, patch.componentOrder[direction]);
        if(patch.groupOrder) next.groups = reorderEntityList(next.groups, patch.groupOrder[direction]);
        if(patch.roots) next.roots = cloneDeep(patch.roots[direction]);
        if(patch.dialogueLogic) next.dialogueLogic = cloneDeep(patch.dialogueLogic[direction]);
        if(patch.controls) next.controls = cloneDeep(patch.controls[direction]);
        if(patch.uniqueTokenCounter) next.uniqueTokenCounter = patch.uniqueTokenCounter[direction];
        next.selection = cloneDeep((patch.selection && patch.selection[direction]) || next.selection || {});
        return next;
    }

    function computeStateAtHistoryIndex(index) {
        if(!historyBaseState) return captureEditorState();
        let state = cloneDeep(historyBaseState);
        const clampedIndex = clamp(index, 0, historyPatches.length);
        for(let i = 0; i < clampedIndex; i++) {
            state = applyHistoryPatchToState(state, historyPatches[i], 'after');
        }
        return state;
    }

    function syncCurrentHistorySelection() {
        if(isRestoringHistory || historyDirty || historyIndex < 0) return;
        const selection = getSelectionSnapshot();
        if(historyIndex === 0 && historyBaseState) {
            historyBaseState.selection = selection;
        } else if(historyIndex > 0 && historyPatches[historyIndex - 1]) {
            historyPatches[historyIndex - 1].selection.after = selection;
        }
        if(historyCurrentState) historyCurrentState.selection = selection;
    }

    function refreshHistoryButtons() {
        const undoBtn = document.getElementById('undo_btn');
        const redoBtn = document.getElementById('redo_btn');
        if(undoBtn) undoBtn.disabled = !historyDirty && historyIndex <= 0;
        if(redoBtn) redoBtn.disabled = historyDirty || historyIndex < 0 || historyIndex >= historyPatches.length;
    }

    function pushHistorySnapshot(reason = '') {
        if(isRestoringHistory) return;
        const state = captureEditorState();
        const serialized = serializeHistoryContent(state);
        if(!historyBaseState) {
            historyBaseState = state;
            historyCurrentState = cloneDeep(state);
            historyCurrentSerialized = serialized;
            historyDirty = false;
            refreshHistoryButtons();
            return;
        }
        if(historyCurrentSerialized === serialized) {
            if(historyCurrentState) historyCurrentState.selection = state.selection;
            syncCurrentHistorySelection();
            historyDirty = false;
            refreshHistoryButtons();
            return;
        }
        historyDirty = false;
        if(historyIndex < historyPatches.length) {
            historyPatches = historyPatches.slice(0, historyIndex);
        }
        const beforeState = historyCurrentState || computeStateAtHistoryIndex(historyIndex);
        const patch = buildHistoryPatch(beforeState, state, reason);
        if(!historyPatchHasContent(patch)) {
            historyCurrentState = cloneDeep(state);
            historyCurrentSerialized = serialized;
            syncCurrentHistorySelection();
            refreshHistoryButtons();
            return;
        }
        historyPatches.push(patch);
        historyIndex = historyPatches.length;
        historyCurrentState = cloneDeep(state);
        historyCurrentSerialized = serialized;
        while(historyPatches.length > HISTORY_LIMIT && historyBaseState) {
            historyBaseState = applyHistoryPatchToState(historyBaseState, historyPatches[0], 'after');
            historyPatches.shift();
            historyIndex = Math.max(0, historyIndex - 1);
        }
        refreshHistoryButtons();
    }

    function markHistoryDirty() {
        if(!isRestoringHistory) {
            historyDirty = true;
            refreshHistoryButtons();
        }
    }

    function flushHistorySnapshot(reason = '') {
        if(!historyDirty) return;
        pushHistorySnapshot(reason);
        historyDirty = false;
    }

    function initializeHistorySnapshot() {
        historyDirty = true;
        flushHistorySnapshot('initial');
    }

    function restoreSelectionSnapshot(selection = {}) {
        const refs = dedupeEntityRefs(selection.selectedEntities || []).filter(ref => !!getEntityByRef(ref));
        const primary = selection.selectedEntity && getEntityByRef(selection.selectedEntity)
            ? { type: selection.selectedEntity.type, id: selection.selectedEntity.id }
            : null;
        if(primary && !refs.some(ref => isSameEntityRef(ref, primary))) refs.push(primary);
        selectedEntities = refs;
        selectedEntity = primary || (refs.length > 0 ? refs[refs.length - 1] : null);
        selectedIds = selectedEntities.filter(item => item.type === 'component').map(item => item.id);
        selectedId = selectedEntity && selectedEntity.type === 'component'
            ? selectedEntity.id
            : (selectedIds.length > 0 ? selectedIds[selectedIds.length - 1] : null);
    }

    function restoreEditorState(state) {
        if(!state) return;
        isRestoringHistory = true;
        try {
            historyDirty = false;
            components = cloneDeep(Array.isArray(state.components) ? state.components : []);
            components.forEach(component => normalizeComponentState(component, { migrateLegacyJoystick: true }));
            groups = cloneDeep(Array.isArray(state.groups) ? state.groups : []).map(normalizeGroupRecord);
            roots = cloneDeep(Array.isArray(state.roots) ? state.roots : []);
            dialogueLogic = normalizeDialogueLogic(state.dialogueLogic);
            uniqueTokenCounter = Number.isFinite(Number(state.uniqueTokenCounter)) ? Number(state.uniqueTokenCounter) : uniqueTokenCounter;
            applyEditorControlState(state.controls || {});
            ensureHierarchyIntegrity();
            restoreSelectionSnapshot(state.selection || {});
            previewAnimRuntime.clear();
            clearPreviewSimulationCaches();
            const wasDirty = historyDirty;
            toggleGridSnapAutoY();
            historyDirty = wasDirty;
            getPersistentAnimSettings();
            updateAspectRatio();
            renderHierarchyPanel();
            updatePropPanel();
            if(workspaceMode === 'blueprint') renderBlueprint(); else renderAll();
        } finally {
            isRestoringHistory = false;
            historyCurrentState = cloneDeep(captureEditorState());
            historyCurrentSerialized = serializeHistoryContent(historyCurrentState);
            refreshHistoryButtons();
        }
    }

    function undoHistory() {
        flushHistorySnapshot('before undo');
        if(historyIndex <= 0) return;
        historyIndex--;
        restoreEditorState(computeStateAtHistoryIndex(historyIndex));
    }

    function redoHistory() {
        flushHistorySnapshot('before redo');
        if(historyIndex < 0 || historyIndex >= historyPatches.length) return;
        historyIndex++;
        restoreEditorState(computeStateAtHistoryIndex(historyIndex));
    }

    const GARBLED_TEXT_REPLACEMENTS = [
        ['\u7EC4\u037C\u01AC', '无图片'],
        ['\u7EC4\u7EC4\u7EC4\u05A1', '无序列帧'],
        ['\u7EC4\uFFFD\u0131\uFFFD', '空文本']
    ];
    const GARBLED_TEXT_FRAGMENT_RE = /[\u0370-\u052F\u0590-\u06FF\uFFFD]+/g;

    function sanitizeGarbledUiText(value, fallback = '') {
        let text = typeof value === 'string' ? value : (value == null ? '' : String(value));
        GARBLED_TEXT_REPLACEMENTS.forEach(([bad, good]) => {
            text = text.split(bad).join(good);
        });
        text = text.replace(GARBLED_TEXT_FRAGMENT_RE, ' ').replace(/\s+/g, ' ').trim();
        if(!text && fallback) return fallback;
        return text;
    }

    function sanitizeGroupDisplayName(value, fallback = '') {
        const cleaned = sanitizeGarbledUiText(value, fallback);
        return cleaned || fallback;
    }

    function createGroupRecord(id = `group_${nextUniqueToken()}`) {
        return {
            id,
            name: `编组 ${id.replace(/^group_/, '')}`,
            children: [],
            visVar: `$grp_show_${id.replace(/^group_/, '').replace(/[^A-Za-z0-9_]+/g, '_')}`,
            visDefault: true,
            bindingEnabled: true,
            pinned: false,
            globalAnim: cloneDeep(GLOBAL_ANIM_DEFAULTS)
        };
    }

    function normalizeGroupRecord(group) {
        const base = group && typeof group === 'object' ? group : {};
        const fallbackName = `编组 ${(base.id || '').replace(/^group_/, '') || nextUniqueToken()}`;
        const normalized = {
            id: base.id || `group_${nextUniqueToken()}`,
            name: sanitizeGroupDisplayName((base.name || '').trim(), fallbackName),
            children: Array.isArray(base.children)
                ? base.children
                    .filter(child => child && (child.type === 'group' || child.type === 'component') && child.id)
                    .map(child => ({ type: child.type, id: child.id }))
                : [],
            visVar: sanitizeIniVarToken(base.visVar, `$grp_show_${(base.id || '').replace(/^group_/, '').replace(/[^A-Za-z0-9_]+/g, '_')}`),
            visDefault: base.visDefault !== false,
            bindingEnabled: base.bindingEnabled !== false,
            pinned: base.pinned === true,
            globalAnim: cloneDeep(base.globalAnim || GLOBAL_ANIM_DEFAULTS)
        };
        normalizeAnimationState(normalized);
        return normalized;
    }

    function getComponentMap() {
        return new Map(components.map(component => [component.id, component]));
    }

    function getGroupMap() {
        return new Map(groups.map(group => [group.id, group]));
    }

    function makeEntityRef(type, id) {
        return type && id ? { type, id } : null;
    }

    function entityRefKey(ref) {
        return ref && ref.type && ref.id ? `${ref.type}:${ref.id}` : '';
    }

    function isSameEntityRef(a, b) {
        return entityRefKey(a) && entityRefKey(a) === entityRefKey(b);
    }

    function dedupeEntityRefs(items) {
        const seen = new Set();
        return (items || []).filter(item => {
            const key = entityRefKey(item);
            if(!key || seen.has(key)) return false;
            seen.add(key);
            return true;
        });
    }

    function getEntityByRef(ref) {
        if(!ref) return null;
        if(ref.type === 'component') return components.find(component => component.id === ref.id) || null;
        if(ref.type === 'group') return groups.find(group => group.id === ref.id) || null;
        return null;
    }

    function getEntityLabel(ref) {
        const entity = getEntityByRef(ref);
        if(!entity) return ref && ref.id ? ref.id : '未知实体';
        if(ref.type === 'group') return sanitizeGroupDisplayName(entity.name, entity.id);
        const typeName = ({
            slider_h: '水平滑条',
            slider_v: '垂直滑条',
            joystick: '摇杆',
            toggle: '开关',
            accum: '积蓄条',
            static: '静态图',
            sequence: '序列动画',
            text: '文本框'
        }[entity.type] || entity.type || '组件');
        return `${typeName} ${entity.id.replace(/^comp_/, '')}`;
    }

    function buildParentLookup() {
        const lookup = new Map();
        groups.forEach(group => {
            (group.children || []).forEach(child => {
                lookup.set(entityRefKey(child), group.id);
            });
        });
        return lookup;
    }

    function getParentGroupIdForEntity(ref, parentLookup = null) {
        const lookup = parentLookup || buildParentLookup();
        return lookup.get(entityRefKey(ref)) || null;
    }

    function getRootRefs() {
        return Array.isArray(roots) ? roots.filter(ref => ref && ref.id && (ref.type === 'group' || ref.type === 'component')) : [];
    }

    function setRootRefs(nextRoots) {
        roots = dedupeEntityRefs(nextRoots || []);
    }

    function ensureHierarchyIntegrity() {
        const componentMap = getComponentMap();
        const groupMap = getGroupMap();
        groups = groups
            .map(normalizeGroupRecord)
            .filter(group => group.id && groupMap.has(group.id));

        const validChildren = new Set();
        groups.forEach(group => {
            const filtered = [];
            (group.children || []).forEach(child => {
                const key = entityRefKey(child);
                if(!key || validChildren.has(key)) return;
                if(child.type === 'component' && componentMap.has(child.id)) {
                    filtered.push({ type: 'component', id: child.id });
                    validChildren.add(key);
                } else if(child.type === 'group' && groups.some(item => item.id === child.id) && child.id !== group.id) {
                    filtered.push({ type: 'group', id: child.id });
                    validChildren.add(key);
                }
            });
            group.children = filtered;
        });

        const explicitRoots = dedupeEntityRefs(getRootRefs()).filter(ref => {
            if(ref.type === 'component') return componentMap.has(ref.id);
            if(ref.type === 'group') return groups.some(group => group.id === ref.id);
            return false;
        });
        const rootKeys = new Set(explicitRoots.map(entityRefKey));

        components.forEach(component => {
            const ref = makeEntityRef('component', component.id);
            const key = entityRefKey(ref);
            if(!validChildren.has(key) && !rootKeys.has(key)) {
                explicitRoots.push(ref);
                rootKeys.add(key);
            }
        });
        groups.forEach(group => {
            const ref = makeEntityRef('group', group.id);
            const key = entityRefKey(ref);
            if(!validChildren.has(key) && !rootKeys.has(key)) {
                explicitRoots.push(ref);
                rootKeys.add(key);
            }
        });

        roots = explicitRoots;
    }

    function getGroupById(groupId) {
        return groups.find(group => group.id === groupId) || null;
    }

    function getComponentById(componentId) {
        return components.find(component => component.id === componentId) || null;
    }

    function getChildrenOfGroup(groupId) {
        const group = getGroupById(groupId);
        return group ? (group.children || []) : [];
    }

    function collectDescendantComponentIds(ref, acc = []) {
        if(!ref) return acc;
        if(ref.type === 'component') {
            acc.push(ref.id);
            return acc;
        }
        const group = getGroupById(ref.id);
        if(!group) return acc;
        (group.children || []).forEach(child => collectDescendantComponentIds(child, acc));
        return acc;
    }

    function getDescendantComponents(ref) {
        return collectDescendantComponentIds(ref, [])
            .map(getComponentById)
            .filter(Boolean);
    }

    function getEntityBounds(ref) {
        if(!ref) return null;
        if(ref.type === 'component') {
            const component = getComponentById(ref.id);
            if(!component) return null;
            return {
                x: Number(component.x) || 0,
                y: Number(component.y) || 0,
                w: Number(component.w) || 0,
                h: Number(component.h) || 0
            };
        }
        const items = getDescendantComponents(ref);
        return getNormalizedBoundsForComponents(items);
    }

    function getAncestorGroupIdsForEntity(ref, parentLookup = null) {
        const lookup = parentLookup || buildParentLookup();
        const ids = [];
        let current = getParentGroupIdForEntity(ref, lookup);
        while(current) {
            ids.unshift(current);
            current = lookup.get(`group:${current}`) || null;
        }
        return ids;
    }

    function getNearestGroupIdForComponent(component) {
        if(!component) return null;
        const lookup = buildParentLookup();
        return lookup.get(`component:${component.id}`) || null;
    }

    function getTopmostAnimatingGroup(component) {
        if(!component) return null;
        const parentLookup = buildParentLookup();
        const ancestorIds = getAncestorGroupIdsForEntity(makeEntityRef('component', component.id), parentLookup);
        for(let i = 0; i < ancestorIds.length; i++) {
            const group = getGroupById(ancestorIds[i]);
            if(group && group.globalAnim && group.globalAnim.mode && group.globalAnim.mode !== 'none') return group;
        }
        return null;
    }

    function getOwningGroupForComponent(component) {
        const groupId = getNearestGroupIdForComponent(component);
        return groupId ? getGroupById(groupId) : null;
    }

    function getEffectiveGroupBindingEnabled(component) {
        if(!component) return true;
        const parentLookup = buildParentLookup();
        const ancestorIds = getAncestorGroupIdsForEntity(makeEntityRef('component', component.id), parentLookup);
        return ancestorIds.every(groupId => {
            const group = getGroupById(groupId);
            return !group || group.bindingEnabled !== false;
        });
    }

    function getEffectiveGroupPinned(component) {
        if(!component) return false;
        const parentLookup = buildParentLookup();
        const ancestorIds = getAncestorGroupIdsForEntity(makeEntityRef('component', component.id), parentLookup);
        return ancestorIds.some(groupId => {
            const group = getGroupById(groupId);
            return !!(group && group.pinned);
        });
    }

    function getVisibilityGroupChain(component) {
        if(!component) return [];
        const parentLookup = buildParentLookup();
        const ancestorIds = getAncestorGroupIdsForEntity(makeEntityRef('component', component.id), parentLookup);
        return ancestorIds
            .map(getGroupById)
            .filter(group => group && group.visVar);
    }

    function getGroupRuntimeVarSuffix(groupId) {
        return String(groupId || '').replace(/^group_/, '').replace(/[^A-Za-z0-9_]+/g, '_');
    }

    function getGroupBindingVarName(groupId) {
        return `$grp_bind_mode_${getGroupRuntimeVarSuffix(groupId)}`;
    }

    function getGroupPinVarName(groupId) {
        return `$grp_pin_mode_${getGroupRuntimeVarSuffix(groupId)}`;
    }

    function getGroupVisibilityConditionExpr(component) {
        const chain = getVisibilityGroupChain(component);
        if(chain.length === 0) return '';
        return chain.map(group => `${group.visVar} == 1`).join(' && ');
    }

    function getDefaultTextVisibilityVar(component) {
        const suffix = String(component && component.id || 'text')
            .replace(/^comp_/, '')
            .replace(/[^A-Za-z0-9_]+/g, '_');
        return `$text_show_${suffix || 'text'}`;
    }

    function getTextVisibilityVar(component) {
        if(!component || component.type !== 'text') return '';
        return sanitizeIniVarToken(component.visVar, getDefaultTextVisibilityVar(component));
    }

    function getTextClickVar(component) {
        if(!component || component.type !== 'text') return '';
        return sanitizeIniVarToken(component.textClickVar, '');
    }

    function isTextVisibilityEnabled(component) {
        if(!(component && component.type === 'text')) return false;
        if(component.textVisibilityEnabled === true) return true;
        if(!dialogueLogic) return false;
        if((dialogueLogic.main && dialogueLogic.main.nodes || []).some(node => node.type === 'text' && node.config && node.config.componentId === component.id)) return true;
        return [dialogueLogic.main, ...(dialogueLogic.dialogues || [])].some(graph => (graph.nodes || []).some(node =>
            (node.type === 'step' && (node.config && node.config.textIds || []).includes(component.id)) ||
            (node.type === 'random' && node.config && (node.config.sourceTextId === component.id || (node.config.branches || []).some(branch => branch.targetTextId === component.id)))
        ));
    }

    function getTextVisibilityConditionExpr(component) {
        return isTextVisibilityEnabled(component) ? `${getTextVisibilityVar(component)} == 1` : '';
    }

    function joinIniConditions(...conditions) {
        return conditions.map(value => String(value || '').trim()).filter(Boolean).join(' && ');
    }

    function getGroupBindingConditionExpr(component) {
        if(!component) return '1';
        const parentLookup = buildParentLookup();
        const ancestorIds = getAncestorGroupIdsForEntity(makeEntityRef('component', component.id), parentLookup);
        if(ancestorIds.length === 0) return '1';
        return ancestorIds.map(groupId => `${getGroupBindingVarName(groupId)} == 1`).join(' && ');
    }

    function getGroupPinnedConditionExpr(component) {
        if(!component) return '0';
        const parentLookup = buildParentLookup();
        const ancestorIds = getAncestorGroupIdsForEntity(makeEntityRef('component', component.id), parentLookup);
        if(ancestorIds.length === 0) return '0';
        return ancestorIds.map(groupId => `${getGroupPinVarName(groupId)} == 1`).join(' || ');
    }

    function buildExportComponentMeta(parentLookup = null) {
        const lookup = parentLookup || buildParentLookup();
        return components.map((component) => {
            const ancestorIds = getAncestorGroupIdsForEntity(makeEntityRef('component', component.id), lookup);
            const nearestGroupId = ancestorIds.length > 0 ? ancestorIds[ancestorIds.length - 1] : null;
            const nearestGroup = nearestGroupId ? getGroupById(nearestGroupId) : null;
            const topmostGroupId = ancestorIds.length > 0 ? ancestorIds[0] : null;
            const topmostGroup = topmostGroupId ? getGroupById(topmostGroupId) : null;
            const animGroup = getTopmostAnimatingGroup(component);
            return {
                nearestGroup,
                topmostGroup,
                animGroup,
                // 拖拽偏移使用最上层编组，使嵌套子编组跟随最上层移动
                offsetXExpr: topmostGroup ? `$grp_off_x_${getGroupRuntimeVarSuffix(topmostGroup.id)}` : '',
                offsetYExpr: topmostGroup ? `$grp_off_y_${getGroupRuntimeVarSuffix(topmostGroup.id)}` : '',
                pinExpr: getGroupPinnedConditionExpr(component),
                groupVisibilityExpr: getGroupVisibilityConditionExpr(component),
                selfVisibilityExpr: getTextVisibilityConditionExpr(component),
                visibilityExpr: joinIniConditions(getGroupVisibilityConditionExpr(component), getTextVisibilityConditionExpr(component)),
                bindingExpr: getGroupBindingConditionExpr(component)
            };
        });
    }

    function selectEntity(ref, isMulti = false, skipRender = false) {
        flushHistorySnapshot('before selection');
        if(!ref || !ref.type || !ref.id) return;
        if(isMulti) {
            if(selectedEntities.some(item => isSameEntityRef(item, ref))) {
                selectedEntities = selectedEntities.filter(item => !isSameEntityRef(item, ref));
            } else {
                selectedEntities.push({ type: ref.type, id: ref.id });
            }
            selectedEntity = selectedEntities.length > 0 ? selectedEntities[selectedEntities.length - 1] : null;
        } else {
            selectedEntity = { type: ref.type, id: ref.id };
            selectedEntities = [selectedEntity];
        }
        selectedIds = selectedEntities.filter(item => item.type === 'component').map(item => item.id);
        selectedId = selectedEntity && selectedEntity.type === 'component' ? selectedEntity.id : (selectedIds.length > 0 ? selectedIds[selectedIds.length - 1] : null);
        if(!skipRender) renderAll();
        else highlightDOM();
        renderHierarchyPanel();
        updatePropPanel();
        syncCurrentHistorySelection();
    }

    // Shift 加选：把一段实体并入当前选择（并集，不移除已有选择）。
    function selectEntityRange(refs) {
        if(!Array.isArray(refs) || refs.length === 0) return;
        flushHistorySnapshot('before selection');
        selectedEntities = dedupeEntityRefs([...selectedEntities, ...refs.map(ref => makeEntityRef(ref.type, ref.id))]);
        selectedEntity = selectedEntities[selectedEntities.length - 1] || null;
        selectedIds = selectedEntities.filter(item => item.type === 'component').map(item => item.id);
        selectedId = selectedEntity && selectedEntity.type === 'component' ? selectedEntity.id : (selectedIds.length > 0 ? selectedIds[selectedIds.length - 1] : null);
        renderAll();
        renderHierarchyPanel();
        updatePropPanel();
        syncCurrentHistorySelection();
    }

    function clearSelection() {
        flushHistorySnapshot('before clear selection');
        selectedEntities = [];
        selectedEntity = null;
        selectedIds = [];
        selectedId = null;
        syncCurrentHistorySelection();
        refreshResourceWindow(null);
    }

    if(hierarchyPanel) {
        hierarchyPanel.addEventListener('click', (e) => {
            const fold = e.target && e.target.closest ? e.target.closest('.hierarchy-fold') : null;
            if(fold) {
                e.preventDefault();
                e.stopPropagation();
                const foldId = fold.getAttribute('data-entity-id');
                if(foldId) {
                    if(hierarchyCollapsedGroups.has(foldId)) hierarchyCollapsedGroups.delete(foldId);
                    else hierarchyCollapsedGroups.add(foldId);
                    renderHierarchyPanel();
                }
                return;
            }
            const row = e.target && e.target.closest ? e.target.closest('.hierarchy-row') : null;
            if(!row) return;
            const type = row.getAttribute('data-entity-type');
            const id = row.getAttribute('data-entity-id');
            if(!type || !id) return;
            e.preventDefault();
            e.stopPropagation();
            if(e.shiftKey && hierarchySelectAnchor) {
                const rowRefs = [...hierarchyPanel.querySelectorAll('.hierarchy-row')]
                    .map(item => ({ type: item.getAttribute('data-entity-type'), id: item.getAttribute('data-entity-id') }));
                const anchorIdx = rowRefs.findIndex(item => isSameEntityRef(item, hierarchySelectAnchor));
                const targetIdx = rowRefs.findIndex(item => item.type === type && item.id === id);
                if(anchorIdx >= 0 && targetIdx >= 0) {
                    const lo = Math.min(anchorIdx, targetIdx);
                    const hi = Math.max(anchorIdx, targetIdx);
                    selectEntityRange(rowRefs.slice(lo, hi + 1).map(item => makeEntityRef(item.type, item.id)));
                    return;
                }
            }
            selectEntity({ type, id }, !!e.ctrlKey);
            hierarchySelectAnchor = { type, id };
        });

        // 拖拽：把组件/编组拖到编组行上放入编组，拖到空白处移回根层级
        const clearHierarchyDragMarks = () => {
            hierarchyPanel.querySelectorAll('.hierarchy-row.drag-over, .hierarchy-row.dragging').forEach(item => item.classList.remove('drag-over', 'dragging'));
            hierarchyPanel.classList.remove('drop-root');
        };
        const getHierarchyDropTarget = (e) => {
            const row = e.target && e.target.closest ? e.target.closest('.hierarchy-row') : null;
            if(row) {
                const type = row.getAttribute('data-entity-type');
                const id = row.getAttribute('data-entity-id');
                if(type && id) return { kind: 'entity', ref: makeEntityRef(type, id), row };
            }
            return { kind: 'root' };
        };
        const isHierarchyDropAllowed = (target) => {
            if(!hierarchyDragRef || !target) return false;
            if(target.kind === 'root') return true;
            if(isSameEntityRef(target.ref, hierarchyDragRef)) return false;
            if(target.ref.type !== 'group') return false;
            // 禁止把编组拖进自己或自己的后代，防止成环
            if(hierarchyDragRef.type === 'group' && getAncestorGroupIdsForEntity(target.ref).includes(hierarchyDragRef.id)) return false;
            return true;
        };
        hierarchyPanel.addEventListener('dragstart', (e) => {
            const row = e.target && e.target.closest ? e.target.closest('.hierarchy-row') : null;
            if(!row) { e.preventDefault(); return; }
            const type = row.getAttribute('data-entity-type');
            const id = row.getAttribute('data-entity-id');
            if(!type || !id) { e.preventDefault(); return; }
            hierarchyDragRef = makeEntityRef(type, id);
            row.classList.add('dragging');
            if(e.dataTransfer) {
                e.dataTransfer.effectAllowed = 'move';
                try { e.dataTransfer.setData('text/plain', `${type}:${id}`); } catch(err) {}
            }
        });
        hierarchyPanel.addEventListener('dragover', (e) => {
            if(!hierarchyDragRef) return;
            const target = getHierarchyDropTarget(e);
            hierarchyPanel.querySelectorAll('.hierarchy-row.drag-over').forEach(item => item.classList.remove('drag-over'));
            hierarchyPanel.classList.remove('drop-root');
            if(!isHierarchyDropAllowed(target)) {
                if(e.dataTransfer) e.dataTransfer.dropEffect = 'none';
                return;
            }
            e.preventDefault();
            if(e.dataTransfer) e.dataTransfer.dropEffect = 'move';
            if(target.kind === 'entity') target.row.classList.add('drag-over');
            else hierarchyPanel.classList.add('drop-root');
        });
        hierarchyPanel.addEventListener('drop', (e) => {
            if(!hierarchyDragRef) return;
            const target = getHierarchyDropTarget(e);
            const allowed = isHierarchyDropAllowed(target);
            e.preventDefault();
            e.stopPropagation();
            const draggedRef = hierarchyDragRef;
            hierarchyDragRef = null;
            clearHierarchyDragMarks();
            if(!allowed) return;
            markHistoryDirty();
            removeChildFromParent(draggedRef);
            if(target.kind === 'entity') {
                const targetGroup = getGroupById(target.ref.id);
                insertChildrenAtParent(target.ref.id, (targetGroup && (targetGroup.children || []).length) || 0, [draggedRef]);
                hierarchyCollapsedGroups.delete(target.ref.id);
            } else {
                insertChildrenAtParent(null, getRootRefs().length, [draggedRef]);
            }
            ensureHierarchyIntegrity();
            renderHierarchyPanel();
            renderAll();
            selectEntity(draggedRef);
        });
        hierarchyPanel.addEventListener('dragend', () => {
            hierarchyDragRef = null;
            clearHierarchyDragMarks();
        });
    }

    // 层级侧栏宽度拖拽调整
    {
        const hierarchyResizer = document.getElementById('hierarchyResizer');
        if(hierarchyResizer) {
            hierarchyResizer.addEventListener('mousedown', (e) => {
                e.preventDefault();
                e.stopPropagation();
                const sidebar = hierarchyResizer.closest('.hierarchy-sidebar');
                if(!sidebar) return;
                const startX = e.clientX;
                const startWidth = sidebar.getBoundingClientRect().width;
                sidebar.style.minWidth = '0px';
                sidebar.style.maxWidth = 'none';
                hierarchyResizer.classList.add('active');
                const onMove = (ev) => {
                    const maxWidth = Math.max(240, Math.min(640, window.innerWidth * 0.5));
                    const nextWidth = clamp(startWidth + (startX - ev.clientX), 180, maxWidth);
                    sidebar.style.width = `${nextWidth}px`;
                    resize();
                };
                const onUp = () => {
                    hierarchyResizer.classList.remove('active');
                    document.removeEventListener('mousemove', onMove);
                    document.removeEventListener('mouseup', onUp);
                };
                __onDocument('mousemove', onMove);
                __onDocument('mouseup', onUp);
            });
        }
    }

    function selectedObj() {
        if(!selectedEntity || selectedEntity.type !== 'component') return null;
        return getComponentById(selectedEntity.id);
    }

    function selectedGroup() {
        if(!selectedEntity || selectedEntity.type !== 'group') return null;
        return getGroupById(selectedEntity.id);
    }

    function getSelectedComponentRefs() {
        return selectedEntities.filter(ref => ref.type === 'component');
    }

    function getSelectedGroupRefs() {
        return selectedEntities.filter(ref => ref.type === 'group');
    }

    function getEntityDragComponentSet(ref) {
        if(!ref) return [];
        if(ref.type === 'component') return [ref.id];
        return collectDescendantComponentIds(ref, []);
    }

    function updateGroupOverlayPositions() {
        groups.forEach(group => {
            const overlay = document.getElementById(group.id);
            if(!overlay) return;
            const bounds = getGroupNormalizedBounds(group.id);
            if(!bounds) return;
            overlay.style.left = (bounds.x * 100) + '%';
            overlay.style.top = (bounds.y * 100) + '%';
            overlay.style.width = (bounds.w * 100) + '%';
            overlay.style.height = (bounds.h * 100) + '%';
        });
    }

    function removeChildFromParent(ref) {
        const key = entityRefKey(ref);
        groups.forEach(group => {
            group.children = (group.children || []).filter(child => entityRefKey(child) !== key);
        });
        roots = getRootRefs().filter(root => entityRefKey(root) !== key);
    }

    function insertChildrenAtParent(parentGroupId, index, refs) {
        const normalizedRefs = dedupeEntityRefs(refs || []);
        if(parentGroupId) {
            const parent = getGroupById(parentGroupId);
            if(!parent) return;
            const before = (parent.children || []).slice(0, index);
            const after = (parent.children || []).slice(index);
            parent.children = dedupeEntityRefs([...before, ...normalizedRefs, ...after]);
        } else {
            const rootList = getRootRefs();
            const before = rootList.slice(0, index);
            const after = rootList.slice(index);
            roots = dedupeEntityRefs([...before, ...normalizedRefs, ...after]);
        }
    }

    function getSiblingCollectionAndIndex(ref) {
        const parentLookup = buildParentLookup();
        const parentGroupId = getParentGroupIdForEntity(ref, parentLookup);
        if(parentGroupId) {
            const parent = getGroupById(parentGroupId);
            const list = parent ? (parent.children || []) : [];
            return {
                parentGroupId,
                list,
                index: list.findIndex(child => isSameEntityRef(child, ref))
            };
        }
        const rootList = getRootRefs();
        return {
            parentGroupId: null,
            list: rootList,
            index: rootList.findIndex(child => isSameEntityRef(child, ref))
        };
    }

    function renderHierarchyPanel() {
        if(!hierarchyPanel) return;
        const rootList = getRootRefs();
        if(rootList.length === 0) {
            const emptyHtml = `<div class="anim-note">当前没有组件或编组。</div>`;
            if(hierarchyRenderCache !== emptyHtml) {
                hierarchyPanel.innerHTML = emptyHtml;
                hierarchyRenderCache = emptyHtml;
            }
            return;
        }
        const renderBranch = (ref, depth = 0) => {
            const entity = getEntityByRef(ref);
            if(!entity) return '';
            const isSelected = selectedEntities.some(item => isSameEntityRef(item, ref));
            const label = getEntityLabel(ref);
            const typeLabel = ref.type === 'group' ? '组' : '组件';
            const isGroup = ref.type === 'group';
            const childCount = isGroup ? (entity.children || []).length : 0;
            const collapsed = isGroup && hierarchyCollapsedGroups.has(ref.id);
            let html = `<div class="hierarchy-item${isSelected ? ' selected' : ''}" data-entity-type="${ref.type}" data-entity-id="${ref.id}" style="padding-left:${depth * 16}px;">`;
            html += `<button type="button" class="hierarchy-row" draggable="true" data-entity-type="${ref.type}" data-entity-id="${ref.id}">`;
            if(isGroup) html += `<span class="hierarchy-fold" data-entity-id="${ref.id}" title="${collapsed ? '展开' : '折叠'}">${collapsed ? '▸' : '▾'}</span>`;
            html += `<span class="hierarchy-badge">${typeLabel}</span><span class="hierarchy-label">${label}${collapsed && childCount > 0 ? `（${childCount}）` : ''}</span>`;
            html += `</button></div>`;
            if(isGroup && !collapsed) {
                const children = entity.children || [];
                children.forEach(child => {
                    html += renderBranch(child, depth + 1);
                });
            }
            return html;
        };
        const nextHtml = rootList.map(ref => renderBranch(ref, 0)).join('');
        if(hierarchyRenderCache !== nextHtml) {
            hierarchyPanel.innerHTML = nextHtml;
            hierarchyRenderCache = nextHtml;
        }
    }

    const PRESET_TOOL_WINDOWS = Object.freeze({
        settings: 'settingsWindow',
        animation: 'animColumn',
        properties: 'componentPanelWrap',
        resources: 'resourceWindow',
        textLogic: 'textLogicWindow'
    });

    function capturePresetEditorLayout() {
        const windows = {};
        Object.entries(PRESET_TOOL_WINDOWS).forEach(([name, id]) => {
            const element = document.getElementById(id); if(!element) return;
            // 记录相对页面根(.ui-builder-app)的坐标,与 absolute 定位一致
            const rect = element.getBoundingClientRect(), computed = window.getComputedStyle(element);
            const hostRect = (element.offsetParent || root).getBoundingClientRect();
            windows[name] = {
                visible: computed.display !== 'none',
                left: Math.round(rect.left - hostRect.left),
                top: Math.round(rect.top - hostRect.top),
                zIndex: Number(computed.zIndex) || 0
            };
        });
        const hierarchy = document.querySelector('.hierarchy-sidebar');
        return {
            uiScale: Number.parseFloat(getComputedStyle(root).getPropertyValue('--ui-scale')) || 1,
            theme: root.classList.contains('light-mode') ? 'light' : 'dark',
            workspaceMode: workspaceMode === 'blueprint' ? 'blueprint' : 'layout',
            blueprintScopeId,
            selectedBlueprintNodeIds: cloneDeep(selectedBlueprintNodeIds),
            selection: getSelectionSnapshot(),
            animationEnabled: isAnimationPanelEnabled(),
            hierarchyVisible: !!(hierarchy && getComputedStyle(hierarchy).display !== 'none'),
            windows
        };
    }

    function applyPresetEditorLayout(layout) {
        if(!layout || typeof layout !== 'object') return false;
        const uiScale = Math.max(.8, Math.min(1.5, Number(layout.uiScale) || 1));
        updateUIScale(uiScale);
        const scaleInput = document.getElementById('ui_scale_input'); if(scaleInput) scaleInput.value = uiScale;
        root.classList.toggle('light-mode', layout.theme === 'light');
        root.classList.toggle('dark-mode', layout.theme !== 'light');
        const animationInput = document.getElementById('show_anim_panel'); if(animationInput) animationInput.checked = layout.animationEnabled === true;
        restoreSelectionSnapshot(layout.selection || {});
        const requestedScope = String(layout.blueprintScopeId || 'main');
        blueprintScopeId = requestedScope === 'main' || dialogueLogic.dialogues.some(item => item.id === requestedScope) ? requestedScope : 'main';
        const activeGraph = getBlueprintGraph();
        const validBlueprintNodeIds = new Set((activeGraph && activeGraph.nodes || []).map(node => node.id));
        selectedBlueprintNodeIds = [...new Set(Array.isArray(layout.selectedBlueprintNodeIds) ? layout.selectedBlueprintNodeIds : [])]
            .filter(id => validBlueprintNodeIds.has(id));
        workspaceMode = layout.workspaceMode === 'blueprint' ? 'blueprint' : 'layout';
        dialogueRuntime = null; root.classList.remove('preview-running');
        document.getElementById('blueprintWorkspace').style.display = workspaceMode === 'blueprint' ? 'block' : 'none';
        workArea.style.display = workspaceMode === 'layout' ? 'block' : 'none';
        document.getElementById('runtimeInspector').style.display = 'none';
        syncWorkspaceModeButtons();
        updatePropPanel();
        refreshAnimationPanelVisibility(selectedObj());
        const hierarchy = document.querySelector('.hierarchy-sidebar');
        if(hierarchy) hierarchy.style.display = layout.hierarchyVisible === false ? 'none' : 'flex';
        let restoredMaxZIndex = toolWindowZIndex;
        // 悬浮窗 absolute 定位,包含块为页面根:按页面尺寸钳制恢复位置
        const presetHostWidth = root.clientWidth || window.innerWidth;
        const presetHostHeight = root.clientHeight || window.innerHeight;
        Object.entries(PRESET_TOOL_WINDOWS).forEach(([name, id]) => {
            const element = document.getElementById(id), saved = layout.windows && layout.windows[name]; if(!element || !saved) return;
            const maxLeft = Math.max(8, presetHostWidth - Math.max(120, element.offsetWidth));
            const maxTop = Math.max(8, presetHostHeight - 80);
            if(Number.isFinite(Number(saved.left))) element.style.left = `${Math.max(8, Math.min(maxLeft, Number(saved.left)))}px`;
            if(Number.isFinite(Number(saved.top))) element.style.top = `${Math.max(8, Math.min(maxTop, Number(saved.top)))}px`;
            if(Number.isFinite(Number(saved.zIndex)) && Number(saved.zIndex) > 0) {
                element.style.zIndex = String(saved.zIndex);
                restoredMaxZIndex = Math.max(restoredMaxZIndex, Number(saved.zIndex));
            }
            const allowed = name !== 'properties' || !!(selectedObj() || selectedGroup());
            const textAllowed = name !== 'textLogic' || !!(selectedObj() && selectedObj().type === 'text');
            if(name !== 'animation') element.style.display = saved.visible && allowed && textAllowed ? 'block' : 'none';
        });
        toolWindowZIndex = restoredMaxZIndex;
        syncWindowDockButtons(); resize();
        if(workspaceMode === 'blueprint') renderBlueprint(); else renderAll();
        return true;
    }

    function buildV64PresetData() {
        const data = {
            version: '66',
            timestamp: Date.now(),
            hash: document.getElementById('char_hash').value,
            matchIndex: document.getElementById('match_index').value,
            matchFirstIndex: document.getElementById('match_first_index').value,
            aspect: document.getElementById('global_aspect').value,
            gridSnapX: document.getElementById('grid_snap_x').value,
            gridSnapY: document.getElementById('grid_snap_y').value,
            gridSnapAutoY: document.getElementById('grid_snap_auto_y').checked,
            shortcuts: getShortcutSettings(),
            persistentAnim: getPersistentAnimSettings(),
            components: cloneDeep(components),
            groups: cloneDeep(groups),
            roots: cloneDeep(roots),
            dialogueLogic: cloneDeep(dialogueLogic),
            uniqueTokenCounter,
            editorLayout: capturePresetEditorLayout(),
            generated: {
                fontAssets: buildFontAssetDataMap(),
                missingAssets: []
            }
        };
        data.components.forEach(component => {
            component.preview = {};
            delete component.groupId;
            delete component.bindingEnabled;
            delete component.pinned;
            delete component._linkedPostActive;
            delete component._linkedPostEnabled;
            if(component.type === 'sequence') component.frames.forEach(frame => { frame.preview = null; });
        });
        data.generated.missingAssets = collectMissingEmbeddedAssets(data.components);
        return data;
    }

    function migrateLegacyPreset(data) {
        const migrationNotes = [];
        components = cloneDeep(Array.isArray(data.components) ? data.components : []);
        components.forEach(component => normalizeComponentState(component, { migrateLegacyJoystick: true }));
        groups = [];
        roots = [];
        dialogueLogic = createDefaultDialogueLogic();

        const legacyGroups = new Map();
        components.forEach(component => {
            const gid = component.groupId || null;
            if(!gid) return;
            if(!legacyGroups.has(gid)) legacyGroups.set(gid, []);
            legacyGroups.get(gid).push(component);
        });

        const createSingleComponentGroup = (component, reason) => {
            const group = createGroupRecord();
            group.name = `${getEntityLabel(makeEntityRef('component', component.id))} 组`;
            group.children = [makeEntityRef('component', component.id)];
            group.visVar = component.type === 'text' ? sanitizeIniVarToken(component.visVar, '') : '';
            group.bindingEnabled = component.bindingEnabled !== false;
            group.pinned = component.pinned === true;
            group.globalAnim = cloneDeep(component.globalAnim || GLOBAL_ANIM_DEFAULTS);
            groups.push(normalizeGroupRecord(group));
            roots.push(makeEntityRef('group', group.id));
            migrationNotes.push(reason || `组件 ${component.id} 被包装为单组件组。`);
        };

        const wrapLegacyBucket = (gid, items) => {
            const bucket = items.slice();
            const signatureMap = new Map();
            bucket.forEach(component => {
                const signature = JSON.stringify({
                    pinned: component.pinned === true,
                    bindingEnabled: component.bindingEnabled !== false,
                    visVar: component.type === 'text' ? sanitizeIniVarToken(component.visVar, '') : ''
                });
                if(!signatureMap.has(signature)) signatureMap.set(signature, []);
                signatureMap.get(signature).push(component);
            });

            const sharedAnimSource = bucket[0];
            if(signatureMap.size === 1) {
                const group = createGroupRecord(gid);
                const sample = bucket[0];
                group.name = `编组 ${gid.replace(/^group_/, '')}`;
                group.children = bucket.map(component => makeEntityRef('component', component.id));
                group.bindingEnabled = sample.bindingEnabled !== false;
                group.pinned = sample.pinned === true;
                const textVisVars = [...new Set(bucket.filter(component => component.type === 'text').map(component => sanitizeIniVarToken(component.visVar, '')).filter(Boolean))];
                group.visVar = textVisVars.length === 1 ? textVisVars[0] : '';
                group.globalAnim = cloneDeep(sharedAnimSource.globalAnim || GLOBAL_ANIM_DEFAULTS);
                groups.push(normalizeGroupRecord(group));
                roots.push(makeEntityRef('group', group.id));
                return;
            }

            const wrapper = createGroupRecord(gid);
            wrapper.name = `编组 ${gid.replace(/^group_/, '')}`;
            wrapper.globalAnim = cloneDeep(sharedAnimSource.globalAnim || GLOBAL_ANIM_DEFAULTS);
            const childGroups = [];
            Array.from(signatureMap.values()).forEach(cluster => {
                const group = createGroupRecord();
                const sample = cluster[0];
                group.name = `${wrapper.name} 子组 ${childGroups.length + 1}`;
                group.children = cluster.map(component => makeEntityRef('component', component.id));
                group.bindingEnabled = sample.bindingEnabled !== false;
                group.pinned = sample.pinned === true;
                const textVisVars = [...new Set(cluster.filter(component => component.type === 'text').map(component => sanitizeIniVarToken(component.visVar, '')).filter(Boolean))];
                group.visVar = textVisVars.length === 1 ? textVisVars[0] : '';
                groups.push(normalizeGroupRecord(group));
                childGroups.push(makeEntityRef('group', group.id));
            });
            wrapper.children = childGroups;
            groups.push(normalizeGroupRecord(wrapper));
            roots.push(makeEntityRef('group', wrapper.id));
            migrationNotes.push(`旧编组 ${gid} 因成员参数不一致被拆分为 ${childGroups.length} 个子组。`);
        };

        legacyGroups.forEach((items, gid) => wrapLegacyBucket(gid, items));

        components.filter(component => !component.groupId).forEach(component => {
            const needsWrapper = component.pinned === true ||
                component.bindingEnabled === false ||
                (component.type === 'text' && sanitizeIniVarToken(component.visVar, ''));
            if(needsWrapper) {
                createSingleComponentGroup(component, `组件 ${component.id} 因旧参数被自动包装为单组件组。`);
            } else {
                roots.push(makeEntityRef('component', component.id));
            }
        });

        if(data.globalBindingEnabled === false) {
            groups.forEach(group => {
                const componentsInGroup = getDescendantComponents(makeEntityRef('group', group.id));
                const hasBindable = componentsInGroup.some(component => component.type !== 'static' && component.type !== 'text' && component.type !== 'sequence');
                if(hasBindable) group.bindingEnabled = false;
            });
            migrationNotes.push('旧预设的全局参数绑定已迁移为各编组默认关闭。');
        }

        components.forEach(component => {
            delete component.groupId;
            delete component.bindingEnabled;
            delete component.pinned;
            if(component.type === 'text') {
                component.textVisibilityEnabled = false;
                component.visVar = getDefaultTextVisibilityVar(component);
            }
        });

        ensureHierarchyIntegrity();
        return migrationNotes;
    }

    function loadV64Preset(data) {
        components = cloneDeep(Array.isArray(data.components) ? data.components : []);
        components.forEach(component => normalizeComponentState(component, { migrateLegacyJoystick: true }));
        groups = cloneDeep(Array.isArray(data.groups) ? data.groups : []).map(normalizeGroupRecord);
        roots = cloneDeep(Array.isArray(data.roots) ? data.roots : []);
        dialogueLogic = normalizeDialogueLogic(data.dialogueLogic);
        uniqueTokenCounter = Number.isFinite(Number(data.uniqueTokenCounter))
            ? Number(data.uniqueTokenCounter)
            : uniqueTokenCounter;
        ensureHierarchyIntegrity();
        return [];
    }
    const DEFAULT_FX_SHADER_SOURCE = String.raw`// **** UI FX SHADER FOR SHEEN / EDGE GLOW / ACTIVE HIGHLIGHT ****
// Updated for v75

Texture1D<float4> IniParams : register(t120);

#define SIZE IniParams[87].xy
#define TL_POS IniParams[87].zw
#define ROTATION IniParams[86].x
#define ASPECT IniParams[86].y
#define ROUNDNESS IniParams[86].z
#define FX_MODE IniParams[88].x
#define FX_TIME IniParams[88].y
#define FX_INTENSITY IniParams[88].z
#define FX_WIDTH IniParams[88].w
#define FX_COLOR IniParams[89].xyz
#define FX_ROUNDNESS IniParams[89].w
#define FX_ALPHA_MUL IniParams[90].x
#define FX_PHASE IniParams[90].y
#define FX_ROT IniParams[90].z
#define FX_BOOST IniParams[90].w

struct vs2ps {
    float4 pos : SV_Position0;
    float2 uv : TEXCOORD1;
};

#ifdef VERTEX_SHADER
void main(
        out vs2ps output,
        uint vertex : SV_VertexID)
{
    float2 centerUV = TL_POS.xy + SIZE.xy * 0.5;
    float2 hSize = SIZE.xy * 0.5;
    float2 localPos;
    float2 uv;

    switch(vertex) {
        case 0: localPos = float2(hSize.x, -hSize.y); uv = float2(1, 0); break;
        case 1: localPos = float2(hSize.x,  hSize.y); uv = float2(1, 1); break;
        case 2: localPos = float2(-hSize.x, -hSize.y); uv = float2(0, 0); break;
        case 3: localPos = float2(-hSize.x,  hSize.y); uv = float2(0, 1); break;
        default: localPos = float2(0, 0); uv = float2(0, 0); break;
    };

    localPos.x *= ASPECT;

    float c = cos(ROTATION + FX_ROT);
    float s = sin(ROTATION + FX_ROT);
    float2 rotatedPos;
    rotatedPos.x = localPos.x * c - localPos.y * s;
    rotatedPos.y = localPos.x * s + localPos.y * c;
    rotatedPos.x /= ASPECT;

    float2 finalUV = centerUV + rotatedPos;
    output.pos.x = finalUV.x * 2.0 - 1.0;
    output.pos.y = (1.0 - finalUV.y) * 2.0 - 1.0;
    output.pos.zw = float2(0, 1);
    output.uv = uv;
}
#endif

#ifdef PIXEL_SHADER
Texture2D<float4> tex : register(t100);

float resolveCornerRadius(float roundness, float2 halfSize)
{
    float minSide = min(halfSize.x, halfSize.y);
    if (roundness < 0.0) {
        return clamp(-roundness, 0.0, minSide);
    }
    return saturate(roundness) * minSide;
}

float shapeDistance(float2 uv, float roundness)
{
    float2 quadDims = max(float2(SIZE.x * ASPECT, SIZE.y), float2(0.0001, 0.0001));
    float2 halfSize = quadDims * 0.5;
    float radius = resolveCornerRadius(roundness, halfSize);
    float2 p = (uv * 2.0 - 1.0) * halfSize;
    float2 q = abs(p) - (halfSize - radius);
    float outside = length(max(q, 0.0));
    float inside = min(max(q.x, q.y), 0.0);
    return radius - (outside + inside);
}

float roundedShapeMask(float2 uv, float roundness)
{
    float2 quadDims = max(float2(SIZE.x * ASPECT, SIZE.y), float2(0.0001, 0.0001));
    float feather = max(min(quadDims.x, quadDims.y) * 0.01, 0.0005);
    return smoothstep(0.0, feather, shapeDistance(uv, roundness));
}

void main(vs2ps input, out float4 result : SV_Target0)
{
    float2 dims;
    tex.GetDimensions(dims.x, dims.y);
    if (!dims.x || !dims.y) discard;

    float2 uv = saturate(input.uv);
    float4 texel = tex.Load(int3(uv.xy * dims.xy, 0));
    float baseAlpha = max(texel.a, 0.001);
    float shapeMask = roundedShapeMask(uv, ROUNDNESS);
    if (shapeMask <= 0.001) discard;
    float dist = shapeDistance(uv, FX_ROUNDNESS);
    float2 quadDims = max(float2(SIZE.x * ASPECT, SIZE.y), float2(0.0001, 0.0001));
    float shapeScale = min(quadDims.x, quadDims.y) * 0.5;
    float softness = max(shapeScale * 0.028, 0.0006);
    float fillMask = smoothstep(0.0, softness, dist);
    float borderWidth = shapeScale * lerp(0.055, 0.18, saturate(FX_WIDTH));
    float edgeMask = saturate(fillMask - smoothstep(borderWidth, borderWidth + softness, dist));
    float glowRadius = borderWidth + shapeScale * 0.14;
    float glowMask = pow(saturate(1.0 - abs(dist) / max(glowRadius, 0.001)), 2.0);

    float sweepPhase = frac(FX_PHASE);
    float sweepCoord = frac(uv.x * 0.78 + uv.y * 0.56 - sweepPhase);
    float bandWidth = lerp(0.08, 0.22, saturate(FX_WIDTH));
    float sheenBand = pow(saturate(1.0 - abs(sweepCoord - 0.5) / max(bandWidth, 0.001)), 2.2);
    float effectBoost = 0.35 + saturate(FX_BOOST) * 0.95;

    float alpha = 0.0;
    float3 color = FX_COLOR;

    if (FX_MODE < 1.5) {
        float topLift = pow(saturate(1.0 - uv.y), 2.4) * 0.35;
        alpha = fillMask * (sheenBand * (0.45 + effectBoost) + topLift + edgeMask * (0.18 + FX_BOOST * 0.22)) * FX_INTENSITY * baseAlpha * shapeMask;
        color = lerp(FX_COLOR, float3(1.0, 1.0, 1.0), 0.35);
    } else if (FX_MODE < 2.5) {
        alpha = glowMask * FX_INTENSITY * max(baseAlpha, fillMask) * shapeMask;
    } else if (FX_MODE < 3.5) {
        float pulse = 0.72 + 0.28 * sin(FX_TIME * 6.28318 * 0.09);
        alpha = (edgeMask * 1.2 + glowMask * 0.6) * FX_INTENSITY * pulse * max(baseAlpha, fillMask) * shapeMask;
        color = lerp(FX_COLOR, float3(1.0, 1.0, 1.0), 0.18);
    } else if (FX_MODE < 4.5) {
        float2 centered = uv * 2.0 - 1.0;
        float radius = length(centered);
        float ringWidth = max(0.04, 0.06 + FX_WIDTH * 0.14);
        float ringMask = pow(saturate(1.0 - abs(radius - 0.78) / ringWidth), 2.4);
        float angle = atan2(centered.y, centered.x) / 6.28318 + 0.5;
        float arcPhase = frac(angle - FX_PHASE);
        float arcMask = pow(saturate(1.0 - abs(arcPhase - 0.5) / max(0.10 + FX_WIDTH * 0.14, 0.001)), 2.0);
        alpha = ringMask * (0.24 + glowMask * 0.42 + arcMask * (0.9 + FX_BOOST * 0.6)) * FX_INTENSITY * shapeMask;
        color = lerp(FX_COLOR, float3(1.0, 1.0, 1.0), 0.22);
    } else {
        float pulse = 0.58 + 0.42 * sin((FX_TIME * 0.08 + FX_PHASE) * 6.28318);
        alpha = (fillMask * (0.35 + glowMask * 0.65) + glowMask * 0.42) * FX_INTENSITY * pulse * max(baseAlpha, fillMask) * shapeMask * effectBoost;
        color = lerp(FX_COLOR, float3(1.0, 1.0, 1.0), 0.42);
    }

    alpha *= max(FX_ALPHA_MUL, 0.0);
    if (alpha <= 0.001) discard;
    result = float4(saturate(color) * alpha, alpha);
}
#endif
`;
    const shaderTemplateFileName = 'draw_2d.hlsl';
    const shaderTemplateSource = DEFAULT_SHADER_SOURCE;
    const shaderFxFileName = 'draw_2d_fx.hlsl';
    const shaderFxSource = DEFAULT_FX_SHADER_SOURCE;
    const DEFAULT_TOGGLE_STEPS = 5;
    const TOGGLE_MULTI_MODE = '5';
    const TOGGLE_PROGRESS_ASSETS = Object.freeze({
        off: '__toggle_multi_off.png',
        on: '__toggle_multi_on.png'
    });
    const DEFAULT_ASSET_PATHS = Object.freeze({
        staticImg: 'static.png',
        glassPanel: '__glass_panel.png',
        handle: '1.png',
        barFill: '2.png',
        barTrack: '3.png',
        sliderHandle: '__slider_handle.png',
        sliderFill: '__slider_fill.png',
        sliderTrack: '__slider_track.png',
        accumFill: '__accum_fill_red.png',
        accumTrack: '__accum_track_blue.png',
        joystickHandle: '__joystick_handle.png',
        postMarker: '__collision_post_marker.png',
        toggleFrame: '__toggle_frame.png',
        toggleOff: '4_off.png',
        toggleOn: '4_on.png',
        toggleProgressOff: TOGGLE_PROGRESS_ASSETS.off,
        toggleProgressOn: TOGGLE_PROGRESS_ASSETS.on,
        fxWhite: '__fx_white.png',
        // 格子滑条档位刻度专用白图：与 FX 着色器/流光开关无关，始终随打包，
        // 避免关闭“常驻流光与表面高光”后配置仍引用未打包的 __fx_white.png。
        gridTick: '__grid_tick.png'
    });
    const LEGACY_ASSET_PATHS = Object.freeze({
        postMarker: '__post_marker.png'
    });

    function clamp(v, min, max) {
        return Math.min(max, Math.max(min, v));
    }

    function readPersistentAnimNumber(id, min, max, fallback) {
        const input = document.getElementById(id);
        const raw = input ? input.value : fallback;
        const num = Number(raw);
        const safe = Number.isFinite(num) ? clamp(num, min, max) : fallback;
        if(input) input.value = safe;
        return safe;
    }

    function getPersistentAnimSettings() {
        const enabled = getInputChecked('p_anim_persistent_enabled', PERSISTENT_ANIM_DEFAULTS.enabled);
        const speedInput = document.getElementById('p_anim_persistent_speed');
        if(speedInput) speedInput.disabled = !enabled;
        return {
            enabled,
            speedMultiplier: readPersistentAnimNumber('p_anim_persistent_speed', 0, 0.5, PERSISTENT_ANIM_DEFAULTS.speedMultiplier),
            flowSpeedScale: readPersistentAnimNumber('p_anim_persistent_flow_speed', 0, 2, PERSISTENT_ANIM_DEFAULTS.flowSpeedScale)
        };
    }

    function getPersistentAnimSpeedMultiplier() {
        return getPersistentAnimSettings().speedMultiplier;
    }

    function getPersistentFlowSpeedScale() {
        return getPersistentAnimSettings().flowSpeedScale;
    }

    function getPersistentPreviewSheenState() {
        const settings = getPersistentAnimSettings();
        const phase01 = ((getPreviewTimeSeconds() * settings.speedMultiplier) % 1 + 1) % 1;
        return {
            enabled: settings.enabled,
            phase01,
            offsetX: phase01 * 320 - 160,
            opacity: settings.enabled ? 0.18 : 0
        };
    }

    function colorToRgbArray(color, fallback = [1, 1, 1]) {
        if(typeof color !== 'string') return fallback.slice();
        const value = color.trim();
        const shortMatch = /^#([0-9a-f]{3})$/i.exec(value);
        const longMatch = /^#([0-9a-f]{6})$/i.exec(value);
        if(shortMatch) {
            const hex = shortMatch[1];
            return [
                parseInt(hex[0] + hex[0], 16) / 255,
                parseInt(hex[1] + hex[1], 16) / 255,
                parseInt(hex[2] + hex[2], 16) / 255
            ];
        }
        if(longMatch) {
            const hex = longMatch[1];
            return [
                parseInt(hex.slice(0, 2), 16) / 255,
                parseInt(hex.slice(2, 4), 16) / 255,
                parseInt(hex.slice(4, 6), 16) / 255
            ];
        }
        return fallback.slice();
    }

    function isToggleMultiMode(component) {
        return !!component && component.type === 'toggle' && component.paramMode === TOGGLE_MULTI_MODE;
    }

    function normalizeToggleState(component) {
        if(!component || component.type !== 'toggle') return;
        if(component.paramMode !== '1' && component.paramMode !== TOGGLE_MULTI_MODE) component.paramMode = '1';
        const parsedSteps = Math.round(Number(component.toggleSteps));
        component.toggleSteps = clamp(Number.isFinite(parsedSteps) ? parsedSteps : DEFAULT_TOGGLE_STEPS, 1, 32);
        const parsedInitial = Math.round(Number(component.initialValue));
        const maxValue = isToggleMultiMode(component) ? component.toggleSteps : 1;
        component.initialValue = clamp(Number.isFinite(parsedInitial) ? parsedInitial : 0, 0, maxValue);
        component.toggleInvert = component.toggleInvert === true;
    }

    function createSolidColorDataUrl(color, size = 8) {
        const canvas = document.createElement('canvas');
        canvas.width = size;
        canvas.height = size;
        const ctx = canvas.getContext('2d');
        ctx.fillStyle = color;
        ctx.fillRect(0, 0, size, size);
        return canvas.toDataURL('image/png');
    }

    function createRoundedRectDataUrl(stops, options = {}) {
        const size = options.size || 192;
        const stroke = options.stroke || 'rgba(255,255,255,0.16)';
        const outline = options.outline || 'rgba(9,14,24,0.32)';
        const canvas = document.createElement('canvas');
        canvas.width = size;
        canvas.height = size;
        const ctx = canvas.getContext('2d');
        const gradient = ctx.createLinearGradient(0, 0, size, size);
        stops.forEach(([offset, color]) => gradient.addColorStop(offset, color));

        ctx.clearRect(0, 0, size, size);
        ctx.fillStyle = gradient;
        ctx.fillRect(0, 0, size, size);

        const gloss = ctx.createLinearGradient(0, 0, 0, size);
        gloss.addColorStop(0, 'rgba(255,255,255,0.42)');
        gloss.addColorStop(0.22, 'rgba(255,255,255,0.14)');
        gloss.addColorStop(0.6, 'rgba(255,255,255,0.02)');
        gloss.addColorStop(1, 'rgba(255,255,255,0)');
        ctx.fillStyle = gloss;
        ctx.fillRect(0, 0, size, size);

        const innerShade = ctx.createLinearGradient(0, 0, 0, size);
        innerShade.addColorStop(0, 'rgba(255,255,255,0)');
        innerShade.addColorStop(0.62, 'rgba(0,0,0,0.04)');
        innerShade.addColorStop(1, 'rgba(0,0,0,0.16)');
        ctx.fillStyle = innerShade;
        ctx.fillRect(0, 0, size, size);

        ctx.strokeStyle = outline;
        ctx.lineWidth = Math.max(1.5, size * 0.012);
        ctx.strokeRect(ctx.lineWidth * 0.5, ctx.lineWidth * 0.5, size - ctx.lineWidth, size - ctx.lineWidth);

        ctx.strokeStyle = stroke;
        ctx.lineWidth = Math.max(2, size * 0.018);
        ctx.strokeRect(ctx.lineWidth * 0.5, ctx.lineWidth * 0.5, size - ctx.lineWidth, size - ctx.lineWidth);
        return canvas.toDataURL('image/png');
    }

    function createCircleDataUrl(stops, options = {}) {
        const size = options.size || 192;
        const outline = options.outline || 'rgba(16,22,34,0.3)';
        const canvas = document.createElement('canvas');
        canvas.width = size;
        canvas.height = size;
        const ctx = canvas.getContext('2d');
        const radius = size * 0.5;
        const gradient = ctx.createRadialGradient(size * 0.34, size * 0.26, size * 0.08, size * 0.5, size * 0.52, radius);
        stops.forEach(([offset, color]) => gradient.addColorStop(offset, color));
        ctx.clearRect(0, 0, size, size);
        ctx.fillStyle = gradient;
        ctx.fillRect(0, 0, size, size);
        const gloss = ctx.createRadialGradient(size * 0.34, size * 0.24, 0, size * 0.34, size * 0.24, radius * 0.9);
        gloss.addColorStop(0, 'rgba(255,255,255,0.78)');
        gloss.addColorStop(0.22, 'rgba(255,255,255,0.2)');
        gloss.addColorStop(0.48, 'rgba(255,255,255,0.02)');
        gloss.addColorStop(1, 'rgba(255,255,255,0)');
        ctx.fillStyle = gloss;
        ctx.fillRect(0, 0, size, size);
        const rimShade = ctx.createRadialGradient(size * 0.5, size * 0.5, radius * 0.42, size * 0.5, size * 0.5, radius);
        rimShade.addColorStop(0, 'rgba(255,255,255,0)');
        rimShade.addColorStop(0.72, 'rgba(0,0,0,0.04)');
        rimShade.addColorStop(1, 'rgba(0,0,0,0.18)');
        ctx.fillStyle = rimShade;
        ctx.fillRect(0, 0, size, size);
        ctx.strokeStyle = outline;
        ctx.lineWidth = Math.max(1.5, size * 0.012);
        ctx.strokeRect(ctx.lineWidth * 0.5, ctx.lineWidth * 0.5, size - ctx.lineWidth, size - ctx.lineWidth);
        ctx.strokeStyle = options.stroke || 'rgba(255,255,255,0.22)';
        ctx.lineWidth = Math.max(2, size * 0.02);
        ctx.strokeRect(ctx.lineWidth * 0.5, ctx.lineWidth * 0.5, size - ctx.lineWidth, size - ctx.lineWidth);
        return canvas.toDataURL('image/png');
    }

    function createPostMarkerDataUrl(options = {}) {
        const size = options.size || 96;
        const canvas = document.createElement('canvas');
        canvas.width = size;
        canvas.height = size;
        const ctx = canvas.getContext('2d');
        const cx = size * 0.5;
        const cy = size * 0.5;
        const radius = size * 0.18;
        ctx.clearRect(0, 0, size, size);

        const glow = ctx.createRadialGradient(cx, cy, radius * 0.35, cx, cy, radius * 2.8);
        glow.addColorStop(0, 'rgba(255,92,92,0.86)');
        glow.addColorStop(0.38, 'rgba(255,56,56,0.34)');
        glow.addColorStop(1, 'rgba(255,0,0,0)');
        ctx.fillStyle = glow;
        ctx.beginPath();
        ctx.arc(cx, cy, radius * 2.8, 0, Math.PI * 2);
        ctx.fill();

        const fill = ctx.createRadialGradient(cx - radius * 0.35, cy - radius * 0.35, radius * 0.18, cx, cy, radius);
        fill.addColorStop(0, '#fff4f4');
        fill.addColorStop(0.22, '#ff9b9b');
        fill.addColorStop(0.62, '#ff2f2f');
        fill.addColorStop(1, '#9e0000');
        ctx.fillStyle = fill;
        ctx.beginPath();
        ctx.arc(cx, cy, radius, 0, Math.PI * 2);
        ctx.fill();

        ctx.strokeStyle = 'rgba(255,255,255,0.94)';
        ctx.lineWidth = Math.max(1.5, size * 0.03);
        ctx.beginPath();
        ctx.arc(cx, cy, radius - ctx.lineWidth * 0.35, 0, Math.PI * 2);
        ctx.stroke();

        return canvas.toDataURL('image/png');
    }

    function createGlassPanelDataUrl(options = {}) {
        const size = options.size || 256;
        const canvas = document.createElement('canvas');
        canvas.width = size;
        canvas.height = size;
        const ctx = canvas.getContext('2d');

        const bg = ctx.createLinearGradient(0, 0, size, size);
        bg.addColorStop(0, 'rgba(214,232,255,0.26)');
        bg.addColorStop(0.45, 'rgba(132,173,224,0.16)');
        bg.addColorStop(1, 'rgba(34,56,92,0.28)');
        ctx.fillStyle = bg;
        ctx.fillRect(0, 0, size, size);

        for(let i = 0; i < 42; i++) {
            const x = (i * 37) % size;
            const y = (i * 53) % size;
            const w = 28 + (i % 5) * 10;
            const h = 10 + (i % 4) * 6;
            ctx.fillStyle = `rgba(255,255,255,${0.03 + (i % 3) * 0.02})`;
            ctx.fillRect(x - w * 0.5, y - h * 0.5, w, h);
        }

        const gloss = ctx.createLinearGradient(0, 0, 0, size);
        gloss.addColorStop(0, 'rgba(255,255,255,0.42)');
        gloss.addColorStop(0.18, 'rgba(255,255,255,0.12)');
        gloss.addColorStop(0.5, 'rgba(255,255,255,0.04)');
        gloss.addColorStop(1, 'rgba(255,255,255,0)');
        ctx.fillStyle = gloss;
        ctx.fillRect(0, 0, size, size);

        ctx.strokeStyle = 'rgba(17,26,40,0.22)';
        ctx.lineWidth = Math.max(1.5, size * 0.012);
        ctx.strokeRect(ctx.lineWidth * 0.5, ctx.lineWidth * 0.5, size - ctx.lineWidth, size - ctx.lineWidth);

        ctx.strokeStyle = 'rgba(255,255,255,0.24)';
        ctx.lineWidth = Math.max(2, size * 0.018);
        ctx.strokeRect(ctx.lineWidth * 0.5, ctx.lineWidth * 0.5, size - ctx.lineWidth, size - ctx.lineWidth);

        return canvas.toDataURL('image/png');
    }

    function drawRoundedRect(ctx, x, y, w, h, r) {
        const radius = Math.min(r, w * 0.5, h * 0.5);
        ctx.beginPath();
        ctx.moveTo(x + radius, y);
        ctx.arcTo(x + w, y, x + w, y + h, radius);
        ctx.arcTo(x + w, y + h, x, y + h, radius);
        ctx.arcTo(x, y + h, x, y, radius);
        ctx.arcTo(x, y, x + w, y, radius);
        ctx.closePath();
    }

    function buildProceduralAssetDataMap() {
        if(buildProceduralAssetDataMap.cache) return buildProceduralAssetDataMap.cache;
        buildProceduralAssetDataMap.cache = {
            [DEFAULT_ASSET_PATHS.staticImg]: createRoundedRectDataUrl([[0, '#8fc7ff'], [0.52, '#3b6eb4'], [1, '#13243d']], { stroke: 'rgba(255,255,255,0.22)', padding: 3, shadowBlur: 18, shadowOffsetY: 8 }),
            [DEFAULT_ASSET_PATHS.glassPanel]: createGlassPanelDataUrl(),
            [DEFAULT_ASSET_PATHS.handle]: createCircleDataUrl([[0, 'rgba(255,255,255,0.95)'], [0.35, 'rgba(217,231,255,0.94)'], [1, 'rgba(113,149,201,0.92)']], { padding: 5 }),
            [DEFAULT_ASSET_PATHS.barFill]: createRoundedRectDataUrl([[0, '#ffd879'], [0.42, '#ff8c6f'], [1, '#cf4a85']], { radius: 40, padding: 3 }),
            [DEFAULT_ASSET_PATHS.barTrack]: createRoundedRectDataUrl([[0, '#65c8ff'], [0.45, '#215e93'], [1, '#112238']], { radius: 40, padding: 3 }),
            [DEFAULT_ASSET_PATHS.sliderHandle]: createCircleDataUrl([[0, '#ffffff'], [0.38, '#dbe8ff'], [1, '#7c9fd5']], { padding: 4 }),
            [DEFAULT_ASSET_PATHS.sliderFill]: createRoundedRectDataUrl([[0, '#ffd879'], [0.42, '#ff8c6f'], [1, '#cf4a85']], { radius: 48, padding: 2 }),
            [DEFAULT_ASSET_PATHS.sliderTrack]: createRoundedRectDataUrl([[0, 'rgba(240,248,255,0.45)'], [0.3, 'rgba(106,151,208,0.28)'], [1, 'rgba(23,37,60,0.68)']], { radius: 48, padding: 2 }),
            [DEFAULT_ASSET_PATHS.accumFill]: createRoundedRectDataUrl([[0, '#ff8d6a'], [0.45, '#e6352e'], [1, '#8f1428']], { radius: 48, padding: 2 }),
            [DEFAULT_ASSET_PATHS.accumTrack]: createRoundedRectDataUrl([[0, '#7fd4ff'], [0.45, '#2d6da8'], [1, '#17324f']], { radius: 48, padding: 2 }),
            [DEFAULT_ASSET_PATHS.joystickHandle]: createCircleDataUrl([[0, '#ffffff'], [0.32, '#dff5ff'], [1, '#5ca6c6']], { padding: 4 }),
            [DEFAULT_ASSET_PATHS.postMarker]: createPostMarkerDataUrl(),
            [LEGACY_ASSET_PATHS.postMarker]: createPostMarkerDataUrl(),
            [DEFAULT_ASSET_PATHS.toggleFrame]: createRoundedRectDataUrl([[0, 'rgba(230,241,255,0.38)'], [0.45, 'rgba(93,133,188,0.26)'], [1, 'rgba(22,34,55,0.7)']], { radius: 36, padding: 3 }),
            [DEFAULT_ASSET_PATHS.toggleOff]: createRoundedRectDataUrl([[0, '#6b7d97'], [0.5, '#2f4055'], [1, '#17202d']], { radius: 36, padding: 3 }),
            [DEFAULT_ASSET_PATHS.toggleOn]: createRoundedRectDataUrl([[0, '#ffe588'], [0.45, '#ff9c66'], [1, '#d9567f']], { radius: 36, padding: 3 }),
            [DEFAULT_ASSET_PATHS.fxWhite]: createSolidColorDataUrl('#ffffff', 16),
            [DEFAULT_ASSET_PATHS.gridTick]: createSolidColorDataUrl('#ffffff', 16),
            [TOGGLE_PROGRESS_ASSETS.off]: createRoundedRectDataUrl([[0, '#6cc4ff'], [0.45, '#1d5f95'], [1, '#0f2336']], { radius: 28 }),
            [TOGGLE_PROGRESS_ASSETS.on]: createRoundedRectDataUrl([[0, '#ffe283'], [0.45, '#ff9368'], [1, '#d44b87']], { radius: 28 })
        };
        return buildProceduralAssetDataMap.cache;
    }

    function getPreviewTextureUrl(component, key) {
        if(!component || !key) return '';
        if(component.preview && component.preview[key]) return component.preview[key];
        const path = component.paths && component.paths[key];
        if(path) {
            const procedural = buildProceduralAssetDataMap();
            if(procedural[path]) return procedural[path];
        }
        return '';
    }

    function getDefaultComponentPaths(type) {
        switch(type) {
            case 'slider_h':
            case 'slider_v':
                return {
                    bg: DEFAULT_ASSET_PATHS.glassPanel,
                    handle: DEFAULT_ASSET_PATHS.sliderHandle,
                    bar_l: DEFAULT_ASSET_PATHS.sliderFill,
                    bar_r: DEFAULT_ASSET_PATHS.sliderTrack
                };
            case 'joystick':
                return {
                    bg: DEFAULT_ASSET_PATHS.glassPanel,
                    handle: DEFAULT_ASSET_PATHS.joystickHandle,
                    post: DEFAULT_ASSET_PATHS.postMarker,
                    bar_l: '',
                    bar_r: ''
                };
            case 'toggle':
                return {
                    off: DEFAULT_ASSET_PATHS.toggleOff,
                    on: DEFAULT_ASSET_PATHS.toggleOn,
                    prog_off: TOGGLE_PROGRESS_ASSETS.off,
                    prog_on: TOGGLE_PROGRESS_ASSETS.on
                };
            case 'accum':
                return {
                    bg: DEFAULT_ASSET_PATHS.glassPanel,
                    bar_l: DEFAULT_ASSET_PATHS.accumFill,
                    bar_r: DEFAULT_ASSET_PATHS.accumTrack
                };
            case 'static':
                return {
                    img: DEFAULT_ASSET_PATHS.staticImg
                };
            case 'sequence':
                return {};
            case 'text':
                return {
                    bg: DEFAULT_ASSET_PATHS.glassPanel
                };
            default:
                return { bg: DEFAULT_ASSET_PATHS.glassPanel };
        }
    }

    function getDefaultResourceOpacity(component, key) {
        return 1;
    }

    function normalizeResourceOpacityValue(component, key, value) {
        const fallback = getDefaultResourceOpacity(component, key);
        const num = Number(value);
        if(!Number.isFinite(num)) return fallback;
        return clamp(num, 0, 1);
    }

    function ensureComponentResourceOpacityState(component) {
        if(!component) return;
        if(!component.resourceOpacity || typeof component.resourceOpacity !== 'object') component.resourceOpacity = {};
        const mergedKeys = new Set([
            ...Object.keys(getDefaultComponentPaths(component.type) || {}),
            ...Object.keys(component.paths || {}),
            ...Object.keys(component.resourceOpacity || {})
        ]);
        mergedKeys.forEach((key) => {
            component.resourceOpacity[key] = normalizeResourceOpacityValue(component, key, component.resourceOpacity[key]);
        });
    }

    function getComponentResourceOpacity(component, key) {
        if(!component || !key) return 1;
        ensureComponentResourceOpacityState(component);
        const opacity = normalizeResourceOpacityValue(component, key, component.resourceOpacity[key]);
        component.resourceOpacity[key] = opacity;
        return opacity;
    }

    function buildResourceAlphaExpr(component, key, baseExpr = '1') {
        const opacity = getComponentResourceOpacity(component, key);
        const opacityExpr = opacity.toFixed(6);
        if(baseExpr === '1') return opacityExpr;
        if(Math.abs(opacity - 1) <= 0.000001) return baseExpr;
        return `(${opacityExpr} * (${baseExpr}))`;
    }

    function createPreviewCanvas(width, height) {
        const canvas = document.createElement('canvas');
        canvas.width = Math.max(1, Math.round(width));
        canvas.height = Math.max(1, Math.round(height));
        return canvas;
    }

    function loadImageElement(src) {
        return new Promise((resolve, reject) => {
            if(!src) {
                resolve(null);
                return;
            }
            const img = new Image();
            img.onload = () => resolve(img);
            img.onerror = () => reject(new Error(`无法加载图片: ${src}`));
            img.src = src;
        });
    }

    function applySurfaceShaderToCanvas(canvas) {
        const ctx = canvas.getContext('2d');
        const { width, height } = canvas;
        if(!width || !height) return canvas;
        const imageData = ctx.getImageData(0, 0, width, height);
        const data = imageData.data;
        const clamp01 = (v) => Math.max(0, Math.min(1, v));
        const smooth = (a, b, x) => {
            if(a === b) return x < a ? 0 : 1;
            const t = clamp01((x - a) / (b - a));
            return t * t * (3 - 2 * t);
        };
        const luminance = (r, g, b) => 0.2126 * r + 0.7152 * g + 0.0722 * b;

        for(let y = 0; y < height; y++) {
            for(let x = 0; x < width; x++) {
                const i = (y * width + x) * 4;
                const a = data[i + 3] / 255;
                if(a <= 0.001) continue;

                const uvx = width > 1 ? x / (width - 1) : 0.5;
                const uvy = height > 1 ? y / (height - 1) : 0.5;
                const cx = uvx * 2 - 1;
                const cy = uvy * 2 - 1;
                const edgeDist = Math.max(Math.abs(cx), Math.abs(cy));
                const edgeMask = smooth(0.58, 0.98, edgeDist);
                const borderMask = smooth(0.82, 1.0, edgeDist);
                const topSpec = Math.pow(clamp01(1 - uvy), 2.7);
                const diagCoord = uvx + uvy;
                const diagBand = smooth(0.16, 0.42, diagCoord) * (1 - smooth(0.52, 0.86, diagCoord));
                const dx = uvx - 0.18;
                const dy = uvy - 0.14;
                const cornerLift = Math.pow(clamp01(1 - Math.sqrt(dx * dx + dy * dy) * 1.45), 2.0);

                let r = data[i] / 255;
                let g = data[i + 1] / 255;
                let b = data[i + 2] / 255;
                const l = clamp01(luminance(r, g, b) * 1.15);
                const edgeTintR = 0.48 + (1.0 - 0.48) * l;
                const edgeTintG = 0.82 + (0.84 - 0.82) * l;
                const edgeTintB = 1.0 + (0.56 - 1.0) * l;

                r += 0.48 * topSpec * 0.16 + edgeTintR * diagBand * 0.12 + 1.0 * cornerLift * 0.10 + edgeTintR * edgeMask * 0.08;
                g += 0.82 * topSpec * 0.16 + edgeTintG * diagBand * 0.12 + 0.84 * cornerLift * 0.10 + edgeTintG * edgeMask * 0.08;
                b += 1.0 * topSpec * 0.16 + edgeTintB * diagBand * 0.12 + 0.56 * cornerLift * 0.10 + edgeTintB * edgeMask * 0.08;

                r = r + (r * 1.08 - r) * borderMask * 0.16;
                g = g + (g * 1.08 - g) * borderMask * 0.16;
                b = b + (b * 1.08 - b) * borderMask * 0.16;

                data[i] = Math.round(clamp01(r) * 255);
                data[i + 1] = Math.round(clamp01(g) * 255);
                data[i + 2] = Math.round(clamp01(b) * 255);
            }
        }

        ctx.putImageData(imageData, 0, 0);
        return canvas;
    }

    async function buildShaderMatchedTexture(src, width, height) {
        if(!src) return '';
        const img = await loadImageElement(src);
        if(!img) return '';
        const canvas = createPreviewCanvas(width, height);
        const ctx = canvas.getContext('2d');
        ctx.clearRect(0, 0, canvas.width, canvas.height);
        ctx.drawImage(img, 0, 0, canvas.width, canvas.height);
        applySurfaceShaderToCanvas(canvas);
        return canvas.toDataURL('image/png');
    }

    function getWorkAreaDesignSize() {
        const aspectInput = document.getElementById('global_aspect') as HTMLInputElement | null;
        const aspect = parseFloat(aspectInput?.value || '') || 1.777;
        return {
            width: BASE_HEIGHT * aspect,
            height: BASE_HEIGHT
        };
    }

    function getWorkAreaPixelSize() {
        const rect = workArea.getBoundingClientRect();
        if(rect.width > 1 && rect.height > 1) {
            return {
                width: rect.width,
                height: rect.height
            };
        }
        return getWorkAreaDesignSize();
    }

    function getComponentPixelRect(component) {
        const area = getWorkAreaPixelSize();
        return {
            width: Math.max(2, Math.round(component.w * area.width)),
            height: Math.max(2, Math.round(component.h * area.height))
        };
    }

    function getComponentHandleMetrics(component) {
        const rect = getComponentPixelRect(component);
        const minSide = Math.max(2, Math.min(rect.width, rect.height));
        const baseSize = Math.round((safeNum(component.handleSize, DEFAULT_HS)) * getWorkAreaPixelSize().width);
        const size = Math.max(2, Math.min(baseSize, Math.round(minSide * 0.92)));
        return {
            width: size,
            height: size,
            radius: Math.round(size * 0.5)
        };
    }

    function getHandlePixelSize(component) {
        const metrics = getComponentHandleMetrics(component);
        return { width: metrics.width, height: metrics.height };
    }

    function getTrackPixelThickness(component) {
        const rect = getComponentPixelRect(component);
        const area = getWorkAreaPixelSize();
        const refSize = Math.max(1, Math.min(rect.width, rect.height, area.width));
        const handle = getComponentHandleMetrics(component);
        return Math.max(1, Math.min(Math.round((safeNum(component.trackThick, DEFAULT_TT)) * area.width), Math.max(1, Math.round(Math.min(handle.width, handle.height) * 0.6)), refSize));
    }

    function getComponentCornerRadiusPx(component) {
        if(!component) return 10;
        const rect = getComponentPixelRect(component);
        const minSide = Math.max(2, Math.min(rect.width, rect.height));
        if(Number.isFinite(Number(component.cornerRadius))) {
            return clamp(Number(component.cornerRadius), 0, minSide * 0.5);
        }
        if(component.type === 'joystick') return Math.max(2, Math.round(minSide * 0.5));
        if(component.type === 'slider_h' || component.type === 'slider_v') return Math.max(2, Math.round(minSide * 0.24));
        if(component.type === 'toggle') return Math.max(3, Math.round(minSide * 0.22));
        if(component.type === 'text') return Math.max(3, Math.round(minSide * 0.12));
        return Math.max(4, Math.round(minSide * 0.14));
    }

    function getComponentFxProfile(component, isSelected = false) {
        const handle = getComponentHandleMetrics(component || {});
        const minSide = Math.max(8, Math.min(handle.width || 24, handle.height || 24, (getComponentPixelRect(component || { w: 0.1, h: 0.1 }).width || 24), (getComponentPixelRect(component || { w: 0.1, h: 0.1 }).height || 24)));
        return {
            sheen: {
                enabled: true,
                color: component && component.type === 'toggle' ? [1.0, 0.86, 0.62] : component && component.type === 'joystick' ? [0.52, 0.88, 1.0] : [0.72, 0.92, 1.0],
                intensity: component && component.type === 'text' ? 0.18 : 0.28,
                width: component && component.type === 'text' ? 0.32 : 0.18,
                roundness: component && component.type === 'joystick' ? 0.9 : component && component.type === 'slider_h' || component && component.type === 'slider_v' ? 0.45 : 0.22
            },
            hoverGlow: {
                enabled: true,
                color: component && component.type === 'toggle' ? [1.0, 0.74, 0.48] : [0.42, 0.82, 1.0],
                intensity: component && component.type === 'text' ? 0.08 : 0.14,
                width: 0.22,
                roundness: component && component.type === 'joystick' ? 0.96 : 0.3
            },
            selectedGlow: {
                enabled: true,
                color: component && component.type === 'toggle' ? [1.0, 0.78, 0.58] : [0.56, 0.9, 1.0],
                intensity: 0.4,
                width: Math.min(0.42, Math.max(0.18, minSide / 160)),
                roundness: component && component.type === 'joystick' ? 0.96 : 0.34
            }
        };
    }

    function getComponentRoundness(component) {
        const rect = getComponentPixelRect(component || { w: 0.1, h: 0.1 });
        const minSide = Math.max(2, Math.min(rect.width, rect.height));
        const radiusPx = getComponentCornerRadiusPx(component);
        return clamp(radiusPx / (minSide * 0.5), 0, 1);
    }

    function getPreviewBackgroundOpacity(component) {
        return getComponentResourceOpacity(component, 'bg');
    }

    function getLocalFlowSpeedScale(modeCodeOrName) {
        return modeCodeOrName === 'shimmer' || modeCodeOrName === 'sheen' || modeCodeOrName === 'radial_sheen' || modeCodeOrName === 2 || modeCodeOrName === 7 || modeCodeOrName === 13
            ? getPersistentFlowSpeedScale()
            : 1;
    }

    function clampAnimNum(value, min, max, fallback) {
        const num = Number(value);
        if(!Number.isFinite(num)) return fallback;
        return clamp(num, min, max);
    }

    function getDockTouchTolerance(anim = GLOBAL_ANIM_DEFAULTS) {
        const trigger = clampAnimNum(anim && anim.trigger, 0.01, 0.4, GLOBAL_ANIM_DEFAULTS.trigger);
        return clamp(Math.max(EDGE_DOCK_TOUCH_EPSILON, trigger * 0.25), EDGE_DOCK_TOUCH_EPSILON, 0.03);
    }

    function getDockActivationDistance(anim = GLOBAL_ANIM_DEFAULTS) {
        return clampAnimNum(anim && anim.trigger, 0.01, 0.4, GLOBAL_ANIM_DEFAULTS.trigger);
    }

    function getEdgeDockVisibleReveal(size, reveal) {
        const span = Math.max(0, Number(size) || 0);
        return Math.min(span, clampAnimNum(reveal, 0.005, 0.3, GLOBAL_ANIM_DEFAULTS.reveal));
    }

    function getEdgeDockClosedBounds(bounds, edge, reveal) {
        if(!bounds || !edge) return bounds ? { ...bounds } : null;
        const closed = {
            x: Number(bounds.x) || 0,
            y: Number(bounds.y) || 0,
            w: Math.max(0, Number(bounds.w) || 0),
            h: Math.max(0, Number(bounds.h) || 0)
        };
        const revealX = getEdgeDockVisibleReveal(closed.w, reveal);
        const revealY = getEdgeDockVisibleReveal(closed.h, reveal);
        if(edge === 'left') closed.x = revealX - closed.w;
        else if(edge === 'right') closed.x = 1 - revealX;
        else if(edge === 'top') closed.y = revealY - closed.h;
        else if(edge === 'bottom') closed.y = 1 - revealY;
        return closed;
    }

    function interpolateNormalizedBounds(fromBounds, toBounds, progress = 1) {
        if(!fromBounds || !toBounds) return null;
        const t = clamp(Number(progress) || 0, 0, 1);
        return {
            x: fromBounds.x + (toBounds.x - fromBounds.x) * t,
            y: fromBounds.y + (toBounds.y - fromBounds.y) * t,
            w: Math.max(0, Number(toBounds.w) || 0),
            h: Math.max(0, Number(toBounds.h) || 0)
        };
    }

    function clipBoundsToViewport(bounds) {
        if(!bounds) return null;
        const left = Math.max(0, Number(bounds.x) || 0);
        const top = Math.max(0, Number(bounds.y) || 0);
        const right = Math.min(1, (Number(bounds.x) || 0) + Math.max(0, Number(bounds.w) || 0));
        const bottom = Math.min(1, (Number(bounds.y) || 0) + Math.max(0, Number(bounds.h) || 0));
        if(right <= left || bottom <= top) return null;
        return {
            x: left,
            y: top,
            w: right - left,
            h: bottom - top
        };
    }

    function getBoundsEnvelope(boundsA, boundsB) {
        if(!boundsA && !boundsB) return null;
        if(!boundsA) return boundsB ? { ...boundsB } : null;
        if(!boundsB) return boundsA ? { ...boundsA } : null;
        const left = Math.min(Number(boundsA.x) || 0, Number(boundsB.x) || 0);
        const top = Math.min(Number(boundsA.y) || 0, Number(boundsB.y) || 0);
        const right = Math.max((Number(boundsA.x) || 0) + Math.max(0, Number(boundsA.w) || 0), (Number(boundsB.x) || 0) + Math.max(0, Number(boundsB.w) || 0));
        const bottom = Math.max((Number(boundsA.y) || 0) + Math.max(0, Number(boundsA.h) || 0), (Number(boundsB.y) || 0) + Math.max(0, Number(boundsB.h) || 0));
        return {
            x: left,
            y: top,
            w: Math.max(0, right - left),
            h: Math.max(0, bottom - top)
        };
    }

    function pointInNormalizedBounds(x, y, bounds) {
        return !!bounds &&
            x >= bounds.x && x <= bounds.x + bounds.w &&
            y >= bounds.y && y <= bounds.y + bounds.h;
    }

    function isPointWithinDockAxisSpan(x, y, bounds, edge, margin = 0) {
        if(!bounds || !edge) return false;
        const pad = Math.max(0, Number(margin) || 0);
        if(edge === 'left' || edge === 'right') {
            return y >= bounds.y - pad && y <= bounds.y + bounds.h + pad;
        }
        return x >= bounds.x - pad && x <= bounds.x + bounds.w + pad;
    }

    function getLocalAnimOptions(componentType) {
        return LOCAL_ANIM_LIBRARY[componentType] || LOCAL_ANIM_LIBRARY.default;
    }

    function getDefaultLocalAnimMode(componentType) {
        const recommended = LOCAL_ANIM_RECOMMENDED[componentType];
        const options = getLocalAnimOptions(componentType);
        if(recommended && options.some(item => item.value === recommended)) return recommended;
        return options && options[0] ? options[0].value : 'none';
    }

    function normalizeAnimationState(component) {
        if(!component) return;
        if(!component.globalAnim || typeof component.globalAnim !== 'object') component.globalAnim = {};
        if(!component.localAnim || typeof component.localAnim !== 'object') component.localAnim = {};
        component.globalAnim.mode = GLOBAL_ANIM_MODE_OPTIONS.some(item => item.value === component.globalAnim.mode) ? component.globalAnim.mode : GLOBAL_ANIM_DEFAULTS.mode;
        component.globalAnim.edge = ['auto', 'left', 'right', 'top', 'bottom'].includes(component.globalAnim.edge) ? component.globalAnim.edge : GLOBAL_ANIM_DEFAULTS.edge;
        component.globalAnim.strength = clampAnimNum(component.globalAnim.strength, 0, 0.5, GLOBAL_ANIM_DEFAULTS.strength);
        component.globalAnim.speed = clampAnimNum(component.globalAnim.speed, 0.001, 0.2, GLOBAL_ANIM_DEFAULTS.speed);
        component.globalAnim.reveal = clampAnimNum(component.globalAnim.reveal, 0.005, 0.3, GLOBAL_ANIM_DEFAULTS.reveal);
        component.globalAnim.trigger = clampAnimNum(component.globalAnim.trigger, 0.01, 0.4, GLOBAL_ANIM_DEFAULTS.trigger);
        component.globalAnim.ease = clampAnimNum(component.globalAnim.ease, 0.02, 1, GLOBAL_ANIM_DEFAULTS.ease);

        const localOptions = getLocalAnimOptions(component.type);
        component.localAnim.mode = localOptions.some(item => item.value === component.localAnim.mode) ? component.localAnim.mode : getDefaultLocalAnimMode(component.type);
        component.localAnim.strength = clampAnimNum(component.localAnim.strength, 0, 1, LOCAL_ANIM_DEFAULTS.strength);
        component.localAnim.speed = clampAnimNum(component.localAnim.speed, 0.001, 0.2, LOCAL_ANIM_DEFAULTS.speed);
    }

    function getGroupAnchorComponent(groupId) {
        if(!groupId) return null;
        const groupItems = getDescendantComponents(makeEntityRef('group', groupId));
        if(groupItems.length === 0) return null;
        return groupItems.slice().sort((a, b) => {
            const az = Number.isFinite(Number(a.zIndex)) ? Number(a.zIndex) : 0;
            const bz = Number.isFinite(Number(b.zIndex)) ? Number(b.zIndex) : 0;
            if(az !== bz) return bz - az;
            return components.indexOf(a) - components.indexOf(b);
        })[0];
    }

    function getGroupNormalizedBounds(groupId) {
        return getEntityBounds(makeEntityRef('group', groupId));
    }

    function getAnimationBounds(component) {
        if(!component) return null;
        const animGroup = getTopmostAnimatingGroup(component);
        if(animGroup) {
            const bounds = getGroupNormalizedBounds(animGroup.id);
            if(bounds) return bounds;
        }
        return {
            x: Number(component.x) || 0,
            y: Number(component.y) || 0,
            w: Number(component.w) || 0,
            h: Number(component.h) || 0
        };
    }

    function resolveDockEdgeForBounds(bounds, preferredEdge = GLOBAL_ANIM_DEFAULTS.edge, tolerance = EDGE_DOCK_TOUCH_EPSILON) {
        if(!bounds) return null;
        const distances = {
            left: bounds.x,
            right: 1 - (bounds.x + bounds.w),
            top: bounds.y,
            bottom: 1 - (bounds.y + bounds.h)
        };
        const edgeMetric = (dist) => dist < 0 ? 0 : dist;
        const touchedEdges = Object.entries(distances)
            .filter(([, dist]) => Number.isFinite(dist) && dist <= tolerance)
            .sort((a, b) => edgeMetric(a[1]) - edgeMetric(b[1]));
        if(touchedEdges.length === 0) return null;
        if(preferredEdge && preferredEdge !== 'auto') {
            return distances[preferredEdge] <= tolerance ? preferredEdge : null;
        }
        return touchedEdges[0][0];
    }

    function getNormalizedBoundsForComponents(items, positions = null) {
        if(!Array.isArray(items) || items.length === 0) return null;
        let minX = Infinity;
        let minY = Infinity;
        let maxX = -Infinity;
        let maxY = -Infinity;
        items.forEach(item => {
            const pos = positions && positions[item.id] ? positions[item.id] : item;
            const x = Number(pos.x) || 0;
            const y = Number(pos.y) || 0;
            const w = Number(item.w) || 0;
            const h = Number(item.h) || 0;
            minX = Math.min(minX, x);
            minY = Math.min(minY, y);
            maxX = Math.max(maxX, x + w);
            maxY = Math.max(maxY, y + h);
        });
        if(!Number.isFinite(minX) || !Number.isFinite(minY) || !Number.isFinite(maxX) || !Number.isFinite(maxY)) return null;
        return {
            x: minX,
            y: minY,
            w: Math.max(0, maxX - minX),
            h: Math.max(0, maxY - minY)
        };
    }

    function normalizedRectsOverlap(a, b, padding = 0.008) {
        if(!a || !b) return false;
        const pad = Math.max(0, Number(padding) || 0);
        return (Number(a.x) || 0) < ((Number(b.x) || 0) + (Number(b.w) || 0) + pad) &&
            ((Number(a.x) || 0) + (Number(a.w) || 0) + pad) > (Number(b.x) || 0) &&
            (Number(a.y) || 0) < ((Number(b.y) || 0) + (Number(b.h) || 0) + pad) &&
            ((Number(a.y) || 0) + (Number(a.h) || 0) + pad) > (Number(b.y) || 0);
    }

    function findComponentSpawnPosition(width, height) {
        const w = clamp(Number(width) || 0.2, 0.04, 0.96);
        const h = clamp(Number(height) || 0.2, 0.04, 0.96);
        const marginX = clamp(Math.min(0.04, Math.max(0.012, w * 0.15)), 0.005, 0.08);
        const marginY = clamp(Math.min(0.04, Math.max(0.012, h * 0.15)), 0.005, 0.08);
        const minX = marginX;
        const minY = marginY;
        const maxX = Math.max(minX, 1 - w - marginX);
        const maxY = Math.max(minY, 1 - h - marginY);
        const stepX = Math.max(0.035, Math.min(0.14, w * 0.75));
        const stepY = Math.max(0.035, Math.min(0.14, h * 0.9));
        const anchor = components.length > 0 ? components[components.length - 1] : null;
        const preferred = {
            x: clamp(anchor ? ((Number(anchor.x) || 0) + stepX) : 0.35, minX, maxX),
            y: clamp(anchor ? ((Number(anchor.y) || 0) + stepY) : 0.35, minY, maxY)
        };
        const candidates = [];
        const seen = new Set();
        const pushCandidate = (x, y) => {
            const cx = clamp(Number(x) || 0, minX, maxX);
            const cy = clamp(Number(y) || 0, minY, maxY);
            const key = `${cx.toFixed(4)}:${cy.toFixed(4)}`;
            if(seen.has(key)) return;
            seen.add(key);
            candidates.push({ x: cx, y: cy });
        };

        pushCandidate(preferred.x, preferred.y);
        pushCandidate(0.35, 0.35);

        const cols = Math.max(1, Math.floor(((maxX - minX) / stepX) + 1.0001));
        const rows = Math.max(1, Math.floor(((maxY - minY) / stepY) + 1.0001));
        for(let diagonal = 0; diagonal <= (cols - 1) + (rows - 1); diagonal++) {
            for(let row = 0; row < rows; row++) {
                const col = diagonal - row;
                if(col < 0 || col >= cols) continue;
                pushCandidate(minX + col * stepX, minY + row * stepY);
            }
        }

        const existingRects = components.map(component => ({
            x: Number(component.x) || 0,
            y: Number(component.y) || 0,
            w: Number(component.w) || 0,
            h: Number(component.h) || 0
        }));
        const padding = Math.max(0.008, Math.min(0.03, Math.min(w, h) * 0.12));
        const preferredRect = { x: preferred.x, y: preferred.y, w, h };
        const ranked = candidates.slice().sort((a, b) => {
            const da = Math.abs(a.x - preferredRect.x) + Math.abs(a.y - preferredRect.y);
            const db = Math.abs(b.x - preferredRect.x) + Math.abs(b.y - preferredRect.y);
            return da - db;
        });

        for(const candidate of ranked) {
            const rect = { x: candidate.x, y: candidate.y, w, h };
            if(!existingRects.some(existing => normalizedRectsOverlap(rect, existing, padding))) {
                return candidate;
            }
        }
        return preferred;
    }

    function getDockSnapAdjustment(bounds, anim) {
        if(!bounds || !anim || anim.mode !== 'edge_dock') return { edge: null, dx: 0, dy: 0 };
        const tolerance = getDockActivationDistance(anim);
        const edge = resolveDockEdgeForBounds(bounds, anim.edge || GLOBAL_ANIM_DEFAULTS.edge, tolerance);
        if(!edge) return { edge: null, dx: 0, dy: 0 };
        if(edge === 'left') return { edge, dx: -bounds.x, dy: 0 };
        if(edge === 'right') return { edge, dx: 1 - (bounds.x + bounds.w), dy: 0 };
        if(edge === 'top') return { edge, dx: 0, dy: -bounds.y };
        return { edge, dx: 0, dy: 1 - (bounds.y + bounds.h) };
    }

    function setDockGuide(edge) {
        if(!workArea) return;
        if(edge) workArea.dataset.dockEdge = edge;
        else delete workArea.dataset.dockEdge;
    }

    function getEffectiveGlobalAnim(component) {
        if(!component) return Object.assign({}, GLOBAL_ANIM_DEFAULTS);
        const inheritedGroup = getTopmostAnimatingGroup(component);
        if(inheritedGroup && inheritedGroup.globalAnim) {
            return inheritedGroup.globalAnim;
        }
        return component.globalAnim || GLOBAL_ANIM_DEFAULTS;
    }

    function getPreviewTimeSeconds() {
        return performance.now() * 0.001;
    }

    function normalizeCursorPoint(clientX, clientY) {
        const rect = workArea.getBoundingClientRect();
        if(!rect.width || !rect.height) return { x: 0.5, y: 0.5 };
        return {
            x: clamp((clientX - rect.left) / rect.width, 0, 1),
            y: clamp((clientY - rect.top) / rect.height, 0, 1)
        };
    }

    const previewPointerState = { x: 0.5, y: 0.5, inside: false, alt: false };
    const previewAnimRuntime = new Map();

    function hasActivePreviewAnimation() {
        if(isDraggingCanvasEntity) return false;
        const persistentAnim = getPersistentAnimSettings();
        if(components.length > 0 && persistentAnim.enabled && persistentAnim.speedMultiplier > 0.000001) return true;
        return components.some(component => {
            const globalMode = getEffectiveGlobalAnim(component).mode || 'none';
            const localMode = (component.localAnim && component.localAnim.mode) || 'none';
            return globalMode !== 'none' ||
                localMode !== 'none' ||
                (componentSupportsPhysics(component) && !!component.physics);
        });
    }

    function ensurePreviewClock() {
        if(previewClockHandle != null) return;
        const tick = () => {
            previewClockHandle = requestAnimationFrame((__ts) => { if (!root.isConnected) return; tick(__ts); });
            renderAll();
        };
        previewClockHandle = requestAnimationFrame((__ts) => { if (!root.isConnected) return; tick(__ts); });
    }

    function syncPreviewClock() {
        if(hasActivePreviewAnimation()) {
            ensurePreviewClock();
        } else if(previewClockHandle != null) {
            cancelAnimationFrame(previewClockHandle);
            previewClockHandle = null;
            previewAnimRuntime.clear();
            clearPreviewSimulationCaches();
        }
    }

    function getGlobalPreviewAnimState(component) {
        const anim = getEffectiveGlobalAnim(component);
        const time = getPreviewTimeSeconds();
        const animGroup = getTopmostAnimatingGroup(component);
        const runtimeKey = animGroup ? `group:${animGroup.id}` : `comp:${component.id}`;
        const prevRuntime = previewAnimRuntime.get(runtimeKey) || { progress: anim.mode === 'edge_dock' ? 0 : 1 };
        const state = {
            translateX: 0,
            translateY: 0,
            scale: 1,
            progress: prevRuntime.progress,
            edge: anim.edge || 'right'
        };
        if(!anim || anim.mode === 'none') return state;
        if(anim.mode === 'group_float_y') {
            state.translateY = Math.sin(time * anim.speed * Math.PI * 2) * anim.strength * getWorkAreaPixelSize().height;
            return state;
        }
        if(anim.mode === 'group_float_x') {
            state.translateX = Math.sin(time * anim.speed * Math.PI * 2) * anim.strength * getWorkAreaPixelSize().width;
            return state;
        }
        if(anim.mode === 'group_pulse') {
            state.scale = 1 + Math.sin(time * anim.speed * Math.PI * 2) * anim.strength * 0.12;
            return state;
        }
        if(anim.mode === 'edge_dock') {
            const bounds = getAnimationBounds(component);
            const pointer = previewPointerState;
            const trigger = getDockActivationDistance(anim);
            const resolvedEdge = resolveDockEdgeForBounds(bounds, anim.edge || GLOBAL_ANIM_DEFAULTS.edge, trigger);
            state.edge = resolvedEdge || (anim.edge || GLOBAL_ANIM_DEFAULTS.edge);
            const ease = clamp(anim.ease || GLOBAL_ANIM_DEFAULTS.ease, 0.02, 1);
            const reveal = clamp(anim.reveal || GLOBAL_ANIM_DEFAULTS.reveal, 0.005, 0.3);
            if(!resolvedEdge || !bounds) {
                const wave = state.progress + (1 - state.progress) * ease;
                state.progress = wave;
                previewAnimRuntime.set(runtimeKey, { progress: wave });
                return state;
            }
            let distance = 1;
            if(pointer.inside) {
                if(resolvedEdge === 'left') distance = pointer.x;
                else if(resolvedEdge === 'right') distance = 1 - pointer.x;
                else if(resolvedEdge === 'top') distance = pointer.y;
                else distance = 1 - pointer.y;
            }
            const hoverOpen = pointer.inside && pointer.alt &&
                distance <= trigger &&
                isPointWithinDockAxisSpan(pointer.x, pointer.y, bounds, resolvedEdge, trigger);
            const closedBounds = getEdgeDockClosedBounds(bounds, resolvedEdge, reveal);
            const currentBounds = interpolateNormalizedBounds(closedBounds, bounds, state.progress);
            const visibleBounds = clipBoundsToViewport(currentBounds);
            let hoverVisible = pointer.inside && pointer.alt && pointInNormalizedBounds(pointer.x, pointer.y, visibleBounds);
            if(!hoverVisible && state.progress > 0.001) {
                const envelopeBounds = clipBoundsToViewport(getBoundsEnvelope(closedBounds, bounds));
                hoverVisible = pointer.inside && pointer.alt && pointInNormalizedBounds(pointer.x, pointer.y, envelopeBounds);
            }
            const openTarget = (hoverOpen || hoverVisible) ? 1 : 0;
            const wave = state.progress + (openTarget - state.progress) * ease;
            state.progress = wave;
            previewAnimRuntime.set(runtimeKey, { progress: wave });
            const area = getWorkAreaPixelSize();
            const nextBounds = interpolateNormalizedBounds(closedBounds, bounds, wave);
            if(nextBounds) {
                state.translateX = (nextBounds.x - bounds.x) * area.width;
                state.translateY = (nextBounds.y - bounds.y) * area.height;
            }
            return state;
        }
        return state;
    }

    function getLocalPreviewAnimState(component) {
        const anim = component && component.localAnim ? component.localAnim : LOCAL_ANIM_DEFAULTS;
        const time = getPreviewTimeSeconds();
        const speed = (anim.speed || LOCAL_ANIM_DEFAULTS.speed) * getLocalFlowSpeedScale(anim.mode);
        const phase01 = ((time * speed) % 1 + 1) % 1;
        const phase = phase01 * Math.PI * 2;
        const wave = Math.sin(phase);
        const waveCos = Math.cos(phase);
        const waveAbs = Math.abs(wave);
        const state = {
            mode: anim.mode || 'none',
            strength: anim.strength || 0,
            phase01,
            scale: 1,
            offsetX: 0,
            offsetY: 0,
            rotate: 0,
            opacity: 1,
            sheenOffset: 0,
            fxBoost: 0,
            fillOpacity: 1,
            handleScale: 1,
            handleOpacity: 1,
            stateOffsetX: 0,
            textWave: 0
        };
        const rect = getComponentPixelRect(component || { w: 0.1, h: 0.1 });
        const amp = state.strength;
        switch(state.mode) {
            case 'breathe':
                state.scale = 1 + wave * amp * 0.08;
                state.opacity = 0.94 + wave * amp * 0.08;
                state.fxBoost = waveAbs * amp * 0.18;
                break;
            case 'type_breathe':
                state.scale = 1 + wave * amp * 0.08;
                state.opacity = 0.94 + wave * amp * 0.08;
                state.fxBoost = waveAbs * amp * 0.16;
                break;
            case 'handle_breathe':
                state.handleScale = 1 + wave * amp * 0.08;
                state.handleOpacity = 0.94 + wave * amp * 0.08;
                state.fxBoost = waveAbs * amp * 0.20;
                break;
            case 'shimmer':
            case 'sheen':
            case 'radial_sheen':
                state.sheenOffset = ((phase01 * 260) % 260) - 130;
                state.fxBoost = state.mode === 'radial_sheen'
                    ? 0.42 + amp * 0.82
                    : state.mode === 'sheen'
                        ? 0.34 + amp * 0.76
                        : 0.28 + amp * 0.72;
                break;
            case 'fill_breathe':
                state.fillOpacity = 0.84 + waveAbs * amp * 0.24;
                state.fxBoost = waveAbs * amp * 0.30;
                break;
            case 'handle_bob':
                if(component && component.type === 'slider_v') state.offsetX = wave * amp * Math.max(4, rect.width * 0.04);
                else state.offsetY = wave * amp * Math.max(4, rect.height * 0.05);
                break;
            case 'gyro_orbit':
                state.offsetX = wave * amp * Math.max(4, rect.width * 0.04);
                state.offsetY = waveCos * amp * Math.max(4, rect.height * 0.04);
                state.rotate = wave * amp * 6;
                state.handleScale = 1 + waveAbs * amp * 0.06;
                state.fxBoost = 0.24 + waveAbs * amp * 0.46;
                break;
            case 'toggle_slide':
                state.stateOffsetX = wave * amp * Math.max(3, rect.width * 0.06);
                state.opacity = 0.94 + waveAbs * amp * 0.08;
                state.fxBoost = waveAbs * amp * 0.32;
                break;
            case 'toggle_pop':
                state.scale = 1 + waveAbs * amp * 0.14;
                state.fxBoost = waveAbs * amp * 0.24;
                break;
            case 'glyph_wave':
                state.textWave = wave * amp * Math.max(2, rect.height * 0.04);
                break;
            case 'glyph_glow':
                state.opacity = 0.96 + waveAbs * amp * 0.06;
                state.fxBoost = 0.38 + waveAbs * amp * 0.82;
                break;
            case 'tilt':
                state.rotate = wave * amp * 5.1566;
                state.offsetY = waveCos * amp * Math.max(2, rect.height * 0.03);
                break;
        }
        return state;
    }

    function getPreviewNodeAnimState(component, localState) {
        const state = Object.assign({
            mode: 'none',
            scale: 1,
            offsetX: 0,
            offsetY: 0,
            rotate: 0,
            opacity: 1
        }, localState || {});
        switch(state.mode) {
            case 'shimmer':
            case 'sheen':
            case 'radial_sheen':
            case 'fill_breathe':
            case 'handle_bob':
            case 'handle_breathe':
            case 'toggle_slide':
            case 'toggle_pop':
            case 'glyph_wave':
            case 'glyph_glow':
                state.scale = 1;
                state.offsetX = 0;
                state.offsetY = 0;
                state.rotate = 0;
                state.opacity = 1;
                break;
            case 'gyro_orbit':
                state.scale = 1;
                state.offsetX = 0;
                state.offsetY = 0;
                state.opacity = 1;
                break;
        }
        return state;
    }

    function getGlobalAnimExportProfile(component) {
        const anim = getEffectiveGlobalAnim(component);
        return {
            mode: anim.mode || 'none',
            modeCode: GLOBAL_ANIM_RUNTIME_MODE[anim.mode || 'none'] || 0,
            edge: anim.edge || GLOBAL_ANIM_DEFAULTS.edge,
            edgeCode: ({ left: 0, right: 1, top: 2, bottom: 3, auto: 4 }[anim.edge || GLOBAL_ANIM_DEFAULTS.edge] ?? 4),
            strength: clampAnimNum(anim.strength, 0, 0.5, GLOBAL_ANIM_DEFAULTS.strength),
            speed: clampAnimNum(anim.speed, 0.001, 0.2, GLOBAL_ANIM_DEFAULTS.speed),
            reveal: clampAnimNum(anim.reveal, 0.005, 0.3, GLOBAL_ANIM_DEFAULTS.reveal),
            trigger: clampAnimNum(anim.trigger, 0.01, 0.4, GLOBAL_ANIM_DEFAULTS.trigger),
            ease: clampAnimNum(anim.ease, 0.02, 1, GLOBAL_ANIM_DEFAULTS.ease),
            touchTolerance: getDockTouchTolerance(anim)
        };
    }

    function getLocalAnimExportProfile(component) {
        const anim = component && component.localAnim ? component.localAnim : LOCAL_ANIM_DEFAULTS;
        return {
            mode: anim.mode || 'none',
            modeCode: LOCAL_ANIM_RUNTIME_MODE[anim.mode || 'none'] || 0,
            strength: clampAnimNum(anim.strength, 0, 1, LOCAL_ANIM_DEFAULTS.strength),
            speed: clampAnimNum(anim.speed, 0.001, 0.2, LOCAL_ANIM_DEFAULTS.speed) * getLocalFlowSpeedScale(anim.mode)
        };
    }

    function buildRuntimeEdgeDockCode(options) {
        const {
            resolvedEdgeVar,
            configEdgeVar,
            xExpr,
            yExpr,
            wExpr,
            hExpr,
            touchTolVar,
            triggerVar,
            easeVar,
            revealVar,
            progressVar,
            txVar,
            tyVar,
            zoomExpr = '1',
            dragLockExpr = '$is_dragging > 0'
        } = options;
        return [
            `        ${resolvedEdgeVar} = -1`,
            `        $d0 = ${zoomExpr}`,
            `        if $d0 < 0.0001`,
            `            $d0 = 0.0001`,
            `        endif`,
            `        $d12 = ${xExpr}`,
            `        $d13 = ${yExpr}`,
            `        $d14 = ${wExpr}`,
            `        $d15 = ${hExpr}`,
            `        $d1 = 0.5 + ((${xExpr}) - 0.5) * $d0`,
            `        $d2 = 0.5 + ((${yExpr}) - 0.5) * $d0`,
            `        $d10 = (${wExpr}) * $d0`,
            `        $d11 = (${hExpr}) * $d0`,
            `        $temp = ${touchTolVar} * $d0`,
            `        if $temp < 0.0005`,
            `            $temp = 0.0005`,
            `        endif`,
            `        $d5 = ${triggerVar} * $d0`,
            `        if $d5 < $temp`,
            `            $d5 = $temp`,
            `        endif`,
            `        $d3 = 2`,
            `        if ${configEdgeVar} == 4`,
            `            $d4 = $d1`,
            `            if $d4 < 0`,
            `                $d4 = 0`,
            `            endif`,
            `            if $d4 <= $d5`,
            `                ${resolvedEdgeVar} = 0`,
            `                $d3 = $d4`,
            `            endif`,
            `            $d4 = 1 - ($d1 + $d10)`,
            `            if $d4 < 0`,
            `                $d4 = 0`,
            `            endif`,
            `            if $d4 <= $d5`,
            `                if $d4 < $d3`,
            `                    ${resolvedEdgeVar} = 1`,
            `                    $d3 = $d4`,
            `                endif`,
            `            endif`,
            `            $d4 = $d2`,
            `            if $d4 < 0`,
            `                $d4 = 0`,
            `            endif`,
            `            if $d4 <= $d5`,
            `                if $d4 < $d3`,
            `                    ${resolvedEdgeVar} = 2`,
            `                    $d3 = $d4`,
            `                endif`,
            `            endif`,
            `            $d4 = 1 - ($d2 + $d11)`,
            `            if $d4 < 0`,
            `                $d4 = 0`,
            `            endif`,
            `            if $d4 <= $d5`,
            `                if $d4 < $d3`,
            `                    ${resolvedEdgeVar} = 3`,
            `                    $d3 = $d4`,
            `                endif`,
            `            endif`,
            `        else if ${configEdgeVar} == 0`,
            `            $d4 = $d1`,
            `            if $d4 < 0`,
            `                $d4 = 0`,
            `            endif`,
            `            if $d4 <= $d5`,
            `                ${resolvedEdgeVar} = 0`,
            `            endif`,
            `        else if ${configEdgeVar} == 1`,
            `            $d4 = 1 - ($d1 + $d10)`,
            `            if $d4 < 0`,
            `                $d4 = 0`,
            `            endif`,
            `            if $d4 <= $d5`,
            `                ${resolvedEdgeVar} = 1`,
            `            endif`,
            `        else if ${configEdgeVar} == 2`,
            `            $d4 = $d2`,
            `            if $d4 < 0`,
            `                $d4 = 0`,
            `            endif`,
            `            if $d4 <= $d5`,
            `                ${resolvedEdgeVar} = 2`,
            `            endif`,
            `        else`,
            `            $d4 = 1 - ($d2 + $d11)`,
            `            if $d4 < 0`,
            `                $d4 = 0`,
            `            endif`,
            `            if $d4 <= $d5`,
            `                ${resolvedEdgeVar} = 3`,
            `            endif`,
            `        endif`,
            `        if ${resolvedEdgeVar} >= 0`,
            `            $d3 = $d5`,
            `            $temp = 0`,
            `            if ${dragLockExpr}`,
            `                $temp = 1`,
            `            else if $dock_modifier == 1`,
            `                $d4 = ${progressVar}`,
            `                if $d4 < 0`,
            `                    $d4 = 0`,
            `                endif`,
            `                if $d4 > 1`,
            `                    $d4 = 1`,
            `                endif`,
            `                $d5 = ${revealVar} * $d0`,
            `                if ${resolvedEdgeVar} == 0`,
            `                    if cursor_x <= $d3`,
            `                        $temp = 1`,
            `                    endif`,
            `                    if cursor_y < ($d2 - $d3)`,
            `                        $temp = 0`,
            `                    endif`,
            `                    if cursor_y > ($d2 + $d11 + $d3)`,
            `                        $temp = 0`,
            `                    endif`,
            `                    if $d5 > $d10`,
            `                        $d5 = $d10`,
            `                    endif`,
            `                    $d6 = ($d5 - $d10) + ($d1 - ($d5 - $d10)) * $d4`,
            `                    $d7 = $d6 + $d10`,
            `                    $d8 = $d2`,
            `                    $d9 = $d8 + $d11`,
            `                else if ${resolvedEdgeVar} == 1`,
            `                    if cursor_x >= (1 - $d3)`,
            `                        $temp = 1`,
            `                    endif`,
            `                    if cursor_y < ($d2 - $d3)`,
            `                        $temp = 0`,
            `                    endif`,
            `                    if cursor_y > ($d2 + $d11 + $d3)`,
            `                        $temp = 0`,
            `                    endif`,
            `                    if $d5 > $d10`,
            `                        $d5 = $d10`,
            `                    endif`,
            `                    $d6 = (1 - $d5) + ($d1 - (1 - $d5)) * $d4`,
            `                    $d7 = $d6 + $d10`,
            `                    $d8 = $d2`,
            `                    $d9 = $d8 + $d11`,
            `                else if ${resolvedEdgeVar} == 2`,
            `                    if cursor_y <= $d3`,
            `                        $temp = 1`,
            `                    endif`,
            `                    if cursor_x < ($d1 - $d3)`,
            `                        $temp = 0`,
            `                    endif`,
            `                    if cursor_x > ($d1 + $d10 + $d3)`,
            `                        $temp = 0`,
            `                    endif`,
            `                    if $d5 > $d11`,
            `                        $d5 = $d11`,
            `                    endif`,
            `                    $d6 = $d1`,
            `                    $d7 = $d6 + $d10`,
            `                    $d8 = ($d5 - $d11) + ($d2 - ($d5 - $d11)) * $d4`,
            `                    $d9 = $d8 + $d11`,
            `                else`,
            `                    if cursor_y >= (1 - $d3)`,
            `                        $temp = 1`,
            `                    endif`,
            `                    if cursor_x < ($d1 - $d3)`,
            `                        $temp = 0`,
            `                    endif`,
            `                    if cursor_x > ($d1 + $d10 + $d3)`,
            `                        $temp = 0`,
            `                    endif`,
            `                    if $d5 > $d11`,
            `                        $d5 = $d11`,
            `                    endif`,
            `                    $d6 = $d1`,
            `                    $d7 = $d6 + $d10`,
            `                    $d8 = (1 - $d5) + ($d2 - (1 - $d5)) * $d4`,
            `                    $d9 = $d8 + $d11`,
            `                endif`,
            `                if $d6 < 0`,
            `                    $d6 = 0`,
            `                endif`,
            `                if $d7 > 1`,
            `                    $d7 = 1`,
            `                endif`,
            `                if $d8 < 0`,
            `                    $d8 = 0`,
            `                endif`,
            `                if $d9 > 1`,
            `                    $d9 = 1`,
            `                endif`,
            `                if $d7 > $d6`,
            `                    if $d9 > $d8`,
            `                        if cursor_x >= $d6`,
            `                            if cursor_x <= $d7`,
            `                                if cursor_y >= $d8`,
            `                                    if cursor_y <= $d9`,
            `                                        $temp = 1`,
            `                                    endif`,
            `                                endif`,
            `                            endif`,
            `                        endif`,
            `                    endif`,
            `                endif`,
            `                if $temp == 0`,
            `                    if $d4 > 0.001`,
            `                        if ${resolvedEdgeVar} == 0`,
            `                            $d6 = $d5 - $d10`,
            `                            $d7 = $d1 + $d10`,
            `                            $d8 = $d2`,
            `                            $d9 = $d2 + $d11`,
            `                        else if ${resolvedEdgeVar} == 1`,
            `                            $d6 = $d1`,
            `                            $d7 = (1 - $d5) + $d10`,
            `                            $d8 = $d2`,
            `                            $d9 = $d2 + $d11`,
            `                        else if ${resolvedEdgeVar} == 2`,
            `                            $d6 = $d1`,
            `                            $d7 = $d1 + $d10`,
            `                            $d8 = $d5 - $d11`,
            `                            $d9 = $d2 + $d11`,
            `                        else`,
            `                            $d6 = $d1`,
            `                            $d7 = $d1 + $d10`,
            `                            $d8 = $d2`,
            `                            $d9 = (1 - $d5) + $d11`,
            `                        endif`,
            `                        if $d6 < 0`,
            `                            $d6 = 0`,
            `                        endif`,
            `                        if $d7 > 1`,
            `                            $d7 = 1`,
            `                        endif`,
            `                        if $d8 < 0`,
            `                            $d8 = 0`,
            `                        endif`,
            `                        if $d9 > 1`,
            `                            $d9 = 1`,
            `                        endif`,
            `                        if $d7 > $d6`,
            `                            if $d9 > $d8`,
            `                                if cursor_x >= $d6`,
            `                                    if cursor_x <= $d7`,
            `                                        if cursor_y >= $d8`,
            `                                            if cursor_y <= $d9`,
            `                                                $temp = 1`,
            `                                            endif`,
            `                                        endif`,
            `                                    endif`,
            `                                endif`,
            `                            endif`,
            `                        endif`,
            `                    endif`,
            `                endif`,
            `            endif`,
            `            if $temp < 0`,
            `                $temp = 0`,
            `            endif`,
            `            if $temp > 1`,
            `                $temp = 1`,
            `            endif`,
            `            ${progressVar} = ${progressVar} + ($temp - ${progressVar}) * ${easeVar}`,
            `            if ${resolvedEdgeVar} == 0`,
            `                $d4 = ${revealVar} * $d0`,
            `                if $d4 > $d10`,
            `                    $d4 = $d10`,
            `                endif`,
            `                ${txVar} = ((($d4 - $d10) - $d1) * (1 - ${progressVar})) / $d0`,
            `            else if ${resolvedEdgeVar} == 1`,
            `                $d4 = ${revealVar} * $d0`,
            `                if $d4 > $d10`,
            `                    $d4 = $d10`,
            `                endif`,
            `                ${txVar} = (((1 - $d4) - $d1) * (1 - ${progressVar})) / $d0`,
            `            else if ${resolvedEdgeVar} == 2`,
            `                $d4 = ${revealVar} * $d0`,
            `                if $d4 > $d11`,
            `                    $d4 = $d11`,
            `                endif`,
            `                ${tyVar} = ((($d4 - $d11) - $d2) * (1 - ${progressVar})) / $d0`,
            `            else`,
            `                $d4 = ${revealVar} * $d0`,
            `                if $d4 > $d11`,
            `                    $d4 = $d11`,
            `                endif`,
            `                ${tyVar} = (((1 - $d4) - $d2) * (1 - ${progressVar})) / $d0`,
            `            endif`,
            `        else`,
            `            ${progressVar} = ${progressVar} + (1 - ${progressVar}) * ${easeVar}`,
            `        endif`
        ].join('\n') + '\n';
    }

    function buildRuntimeEdgeSnapCode(options) {
        const {
            modeVar,
            configEdgeVar,
            offsetXVar,
            offsetYVar,
            xExpr,
            yExpr,
            wExpr,
            hExpr,
            triggerVar,
            zoomExpr = '$zoom_global'
        } = options;
        return [
            `                if $dock_modifier == 1 && ${modeVar} == 1`,
            `                    $d0 = ${zoomExpr}`,
            `                    if $d0 < 0.0001`,
            `                        $d0 = 0.0001`,
            `                    endif`,
            `                    $d1 = 0.5 + ((${xExpr}) - 0.5) * $d0`,
            `                    $d2 = 0.5 + ((${yExpr}) - 0.5) * $d0`,
            `                    $d3 = (${wExpr}) * $d0`,
            `                    $d4 = (${hExpr}) * $d0`,
            `                    $d5 = (${triggerVar}) * $d0`,
            `                    if $d5 < 0.0005`,
            `                        $d5 = 0.0005`,
            `                    endif`,
            `                    $d6 = $d1`,
            `                    if $d6 < 0`,
            `                        $d6 = 0`,
            `                    endif`,
            `                    $d7 = 1 - ($d1 + $d3)`,
            `                    if $d7 < 0`,
            `                        $d7 = 0`,
            `                    endif`,
            `                    $d8 = $d2`,
            `                    if $d8 < 0`,
            `                        $d8 = 0`,
            `                    endif`,
            `                    $d9 = 1 - ($d2 + $d4)`,
            `                    if $d9 < 0`,
            `                        $d9 = 0`,
            `                    endif`,
            `                    $temp = -1`,
            `                    $d10 = 2`,
            `                    if ${configEdgeVar} == 0`,
            `                        if $d6 <= $d5`,
            `                            $temp = 0`,
            `                        endif`,
            `                    else if ${configEdgeVar} == 1`,
            `                        if $d7 <= $d5`,
            `                            $temp = 1`,
            `                        endif`,
            `                    else if ${configEdgeVar} == 2`,
            `                        if $d8 <= $d5`,
            `                            $temp = 2`,
            `                        endif`,
            `                    else if ${configEdgeVar} == 3`,
            `                        if $d9 <= $d5`,
            `                            $temp = 3`,
            `                        endif`,
            `                    else`,
            `                        if $d6 <= $d5`,
            `                            $temp = 0`,
            `                            $d10 = $d6`,
            `                        endif`,
            `                        if $d7 <= $d5`,
            `                            if $d7 < $d10`,
            `                                $temp = 1`,
            `                                $d10 = $d7`,
            `                            endif`,
            `                        endif`,
            `                        if $d8 <= $d5`,
            `                            if $d8 < $d10`,
            `                                $temp = 2`,
            `                                $d10 = $d8`,
            `                            endif`,
            `                        endif`,
            `                        if $d9 <= $d5`,
            `                            if $d9 < $d10`,
            `                                $temp = 3`,
            `                            endif`,
            `                        endif`,
            `                    endif`,
            `                    if $temp == 0`,
            `                        ${offsetXVar} = ${offsetXVar} - ($d1 / $d0)`,
            `                    else if $temp == 1`,
            `                        ${offsetXVar} = ${offsetXVar} + ((1 - ($d1 + $d3)) / $d0)`,
            `                    else if $temp == 2`,
            `                        ${offsetYVar} = ${offsetYVar} - ($d2 / $d0)`,
            `                    else if $temp == 3`,
            `                        ${offsetYVar} = ${offsetYVar} + ((1 - ($d2 + $d4)) / $d0)`,
            `                    endif`,
            `                endif`
        ].join('\n') + '\n';
    }

    function validatePresentIfBalance(iniText) {
        const match = String(iniText || '').match(/\[Present\]\n([\s\S]*?)\n\[CustomShaderDraw\]/);
        if(!match) return;
        const lines = match[1].split('\n');
        let depth = 0;
        for(let i = 0; i < lines.length; i++) {
            const line = lines[i].trim();
            if(!line || line.startsWith(';')) continue;
            if(/^if\b/i.test(line)) depth++;
            else if(/^endif\b/i.test(line)) depth--;
            else if(/^else\b/i.test(line)) {
                if(depth <= 0) {
                    console.error(`Present 第 ${i + 1} 行孤立 else:`, line, `\n上下文:\n`, lines.slice(Math.max(0,i-2), i+3).join('\n'));
                    throw new Error(`生成的 [Present] 在第 ${i + 1} 行出现孤立 else：${line}`);
                }
            }
            if(depth < 0) {
                const ctx = lines.slice(Math.max(0,i-5), i+2).join('\n');
                console.error(`Present 第 ${i + 1} 行孤立 endif:`, line, `\n前5行:\n`, ctx);
                throw new Error(`生成的 [Present] 在第 ${i + 1} 行出现孤立 endif：${line}`);
            }
        }
        if(depth !== 0) {
            throw new Error(`生成的 [Present] if/endif 未平衡，剩余层级 ${depth}`);
        }
    }

    function expandElseIfChains(iniText) {
        const lines = String(iniText || '').split('\n');
        const out = [];
        const stack = [];
        for(const rawLine of lines) {
            const ifMatch = rawLine.match(/^(\s*)if\b(.*)$/i);
            const elseIfMatch = rawLine.match(/^(\s*)else if\b(.*)$/i);
            const elseMatch = rawLine.match(/^(\s*)else\b(.*)$/i);
            const endifMatch = rawLine.match(/^(\s*)endif\b(.*)$/i);
            if(elseIfMatch) {
                const top = stack[stack.length - 1];
                const baseIndent = top ? top.indent : elseIfMatch[1];
                const nestedIndent = baseIndent + '    ';
                out.push(`${baseIndent}else`);
                out.push(`${nestedIndent}if${elseIfMatch[2]}`);
                stack.push({ indent: nestedIndent, synthetic: true });
                continue;
            }
            if(ifMatch) {
                if(/^\s*else\s+if\b/i.test(rawLine)) {
                    // already handled above
                } else {
                    out.push(rawLine);
                    stack.push({ indent: ifMatch[1], synthetic: false });
                }
                continue;
            }
            if(elseMatch) {
                const top = stack[stack.length - 1];
                const indent = top ? top.indent : elseMatch[1];
                out.push(`${indent}else${elseMatch[2] || ''}`);
                continue;
            }
            if(endifMatch) {
                if(stack.length === 0) {
                    out.push(rawLine);
                    continue;
                }
                let frame = stack.pop();
                out.push(`${frame.indent}endif${endifMatch[2] || ''}`);
                while(frame && frame.synthetic) {
                    if(stack.length === 0) break;
                    frame = stack.pop();
                    out.push(`${frame.indent}endif`);
                }
                continue;
            }
            out.push(rawLine);
        }
        return out.join('\n');
    }

    function buildRuntimeWaveSamples(kind, sampleCount = AUTO_FUNCTION_SAMPLE_COUNT) {
        const samples = [];
        for(let idx = 0; idx < sampleCount; idx++) {
            const t = idx / sampleCount;
            let value = 0;
            switch(kind) {
                case 'sine':
                    value = Math.sin(t * Math.PI * 2);
                    break;
                case 'cosine':
                    value = Math.cos(t * Math.PI * 2);
                    break;
                case 'pulse':
                    value = 0.5 + 0.5 * Math.sin(t * Math.PI * 2);
                    break;
                case 'absPulse':
                    value = Math.abs(Math.sin(t * Math.PI * 2));
                    break;
                case 'triangle':
                    value = 1 - Math.abs(((t * 4) % 4) - 2);
                    break;
                default:
                    value = 0;
            }
            samples.push(value);
        }
        return samples;
    }

    function buildAnimationSampleAssignment(goalVar, phaseVar, sampleKind, scaleExpr = '1', biasExpr = '0') {
        const samples = buildRuntimeWaveSamples(sampleKind);
        const count = samples.length;
        let out = '';
        samples.forEach((sample, idx) => {
            const line = `${goalVar} = (${biasExpr}) + (${sample.toFixed(6)}) * (${scaleExpr})`;
            if(idx === 0) out += `        if ${phaseVar} < ${(1 / count).toFixed(6)}\n            ${line}\n`;
            else if(idx < count - 1) out += `        else if ${phaseVar} < ${((idx + 1) / count).toFixed(6)}\n            ${line}\n`;
            else out += `        else\n            ${line}\n`;
        });
        out += `        endif\n`;
        return out;
    }

    const previewInteractiveRuntime = new Map();
    const previewTextVariableStates = new Map();
    const previewAccumState = new Map();
    let previewSimulationSnapshot = null;

    function clearPreviewSimulationCaches() {
        invalidateLinkedValuesCache();
        linkedSmoothValues = new WeakMap();
        previewInteractiveRuntime.clear();
        previewAccumState.clear();
        previewTextVariableStates.clear();
        previewActionVarValues.clear();
        previewRangeActionsInitialized = false;
        // 清除摇杆碰撞桩激活状态
        if(Array.isArray(components)) components.forEach(c => {
            if(!c) return;
            c._linkedPostActive = false;
            c._linkedPostEnabled = false;
            if(Array.isArray(c.linkedSlaves)) {
                c.linkedSlaves.forEach(link => {
                    if(link) {
                        delete link._previewPostActive;
                        delete link._previewPostEverActive;
                        delete link._previewLastMappedValue;
                        delete link._previewRangeState;
                    }
                });
            }
            if(Array.isArray(c.rangeTriggers)) {
                c.rangeTriggers.forEach(trigger => {
                    if(trigger) delete trigger._previewRangeState;
                });
            }
        });
        previewSimulationSnapshot = null;
    }

    function wrap01(value) {
        const num = Number(value) || 0;
        return ((num % 1) + 1) % 1;
    }

    // ========== 嵌套联动（区间映射）工具函数 ==========
    
    /** 获取组件预览值（滑块为标量，摇杆为 {x,y}） */
    function getComponentPreviewRawValue(component) {
        if(!component) return 0;
        if(component.type === 'joystick') {
            const v = getJoystickPreviewVector(component, previewSimulationSnapshot);
            return { x: v.x, y: v.y };
        }
        const sliderValue = getSliderPreviewValue(component, previewSimulationSnapshot);
        return sliderUsesExplicitRange(component)
            ? sliderNormalizedToActual(component, sliderValue)
            : sliderValue;
    }

    /**
     * 根据区间映射计算目标值。
     * 将源值在区间 [srcMin, srcMax] 内归一化到 [0,1]，
     * 再根据溢出模式 / 分边模式 / 目标组件类型进行转换。
     * @param {number} sourceValue - 源组件当前值 (滑块 0~1)
     * @param {number} srcMin - 区间下限
     * @param {number} srcMax - 区间上限
     * @param {string} overflow - 溢出处理: 'reset' 超出归零, 'keep_max' 保持边界
     * @param {string} splitSide - 分边: 'left' 目标左半, 'right' 目标右半, 'both' 全范围
     * @param {object} targetComp - 目标组件对象，用于判断双向/格子模式
     * @returns {number} 映射后的目标值 (普通滑块 0~1, 双向滑块 0.5 为中心)
     */
    function computeLinkedMappedValue(sourceValue, srcMin, srcMax, overflow, splitSide, targetComp) {
        // 防御性默认值: 源值域默认 0~0.5 (旧版兼容)
        const sMin = srcMin ?? 0;
        const sMax = srcMax ?? 0.5;
        const range = sMax - sMin;
        // 区间无效: 双向滑块归位中心 0.5, 普通归零
        if(range <= 0) {
            return (targetComp && targetComp.paramMode === '2') ? 0.5 : 0;
        }
        
        // ---- 超出范围处理 ----
        if(sourceValue < sMin || sourceValue > sMax) {
            if(overflow === 'reset') {
                // 超出归零: 双向归 0.5(中心), 格子归 0, 普通归 0
                if(targetComp && targetComp.paramMode === '2') return 0.5;
                if(isSliderGridMode(targetComp)) return 0;
                return 0;
            } else { // 'keep_max' 保持边界: 低于区间→最小值, 高于区间→最大值
                if(sourceValue < sMin) {
                    if(targetComp && targetComp.paramMode === '2') return 0.5;
                    return 0;  // 低于区间: 双向归 0.5, 普通归 0
                }
                return 1;      // 高于区间: 归最大值 1
            }
        }
        
        // ---- 区间内归一化 ----
        const mapped = (sourceValue - sMin) / range;
        let result = clamp(mapped, 0, 1);
        
        // 格子模式: 吸附到最近格子步进 (2~N 档)
        if(isSliderGridMode(targetComp)) {
            const gridSteps = Math.max(2, targetComp.gridSteps || 3);
            const gridIdx = clamp(Math.round(result * (gridSteps - 1)), 0, gridSteps - 1);
            result = gridIdx / (gridSteps - 1);
        }
        
        // 双向模式: splitSide 决定映射到目标的左/右/全范围
        if(targetComp && targetComp.paramMode === '2') {
            if(splitSide === 'left') {
                result = 0.5 - result * 0.5;  // 归一化 0→0.5(中心), 归一化 1→0(最左)
            } else if(splitSide === 'right') {
                result = 0.5 + result * 0.5;  // 归一化 0→0.5(中心), 归一化 1→1(最右)
            }
            // splitSide === 'both' 时保持 [0,1], 0.5 为自然中心
        }
        
        return result;
    }

    let linkedValuesCache = null;
    function invalidateLinkedValuesCache() {
        linkedValuesCache = null;
    }

    function getLinkedSlaveEffectiveRegionMode(link, srcComponent, targetComponent) {
        const requested = link && (link.regionMode === 'rect' || link.regionMode === 'single') ? link.regionMode : null;
        const srcIsJoystick = srcComponent && srcComponent.type === 'joystick';
        const tgtIsJoystick = targetComponent && targetComponent.type === 'joystick';
        if(srcIsJoystick && tgtIsJoystick) {
            return 'rect';
        }
        return requested || 'single';
    }

    function createDefaultLinkedSlaveRegionPoints() {
        return [
            { x: -0.5, y: 0.5 },
            { x: 0.5, y: 0.5 },
            { x: 0.5, y: -0.5 },
            { x: -0.5, y: -0.5 }
        ];
    }

    function getRangeTriggerRegionPoints(trigger) {
        const fallback = [
            { x: Number.isFinite(Number(trigger && trigger.srcMin)) ? Number(trigger.srcMin) : -0.5, y: Number.isFinite(Number(trigger && trigger.srcMaxY)) ? Number(trigger.srcMaxY) : 0.5 },
            { x: Number.isFinite(Number(trigger && trigger.srcMax)) ? Number(trigger.srcMax) : 0.5, y: Number.isFinite(Number(trigger && trigger.srcMaxY)) ? Number(trigger.srcMaxY) : 0.5 },
            { x: Number.isFinite(Number(trigger && trigger.srcMax)) ? Number(trigger.srcMax) : 0.5, y: Number.isFinite(Number(trigger && trigger.srcMinY)) ? Number(trigger.srcMinY) : -0.5 },
            { x: Number.isFinite(Number(trigger && trigger.srcMin)) ? Number(trigger.srcMin) : -0.5, y: Number.isFinite(Number(trigger && trigger.srcMinY)) ? Number(trigger.srcMinY) : -0.5 }
        ];
        const raw = Array.isArray(trigger && trigger.regionPoints) && trigger.regionPoints.length === 4 ? trigger.regionPoints : fallback;
        return raw.map((point, idx) => {
            const base = fallback[idx];
            const x = Number.isFinite(Number(point && point.x)) ? Number(point.x) : base.x;
            const y = Number.isFinite(Number(point && point.y)) ? Number(point.y) : base.y;
            return {
                x: clamp(x, -1, 1),
                y: clamp(y, -1, 1)
            };
        });
    }

    function getLinkedSlaveRegionPoints(link) {
        const fallback = [
            { x: Number.isFinite(Number(link && link.srcMin)) ? Number(link.srcMin) : -0.5, y: Number.isFinite(Number(link && link.srcMaxY)) ? Number(link.srcMaxY) : 0.5 },
            { x: Number.isFinite(Number(link && link.srcMax)) ? Number(link.srcMax) : 0.5, y: Number.isFinite(Number(link && link.srcMaxY)) ? Number(link.srcMaxY) : 0.5 },
            { x: Number.isFinite(Number(link && link.srcMax)) ? Number(link.srcMax) : 0.5, y: Number.isFinite(Number(link && link.srcMinY)) ? Number(link.srcMinY) : -0.5 },
            { x: Number.isFinite(Number(link && link.srcMin)) ? Number(link.srcMin) : -0.5, y: Number.isFinite(Number(link && link.srcMinY)) ? Number(link.srcMinY) : -0.5 }
        ];
        const raw = Array.isArray(link && link.regionPoints) && link.regionPoints.length === 4 ? link.regionPoints : fallback;
        return raw.map((point, idx) => {
            const base = fallback[idx];
            const x = Number.isFinite(Number(point && point.x)) ? Number(point.x) : base.x;
            const y = Number.isFinite(Number(point && point.y)) ? Number(point.y) : base.y;
            return {
                x: clamp(x, -1, 1),
                y: clamp(y, -1, 1)
            };
        });
    }

    function getLinkedSlaveRegionCenter(points) {
        if(!Array.isArray(points) || points.length !== 4) return { x: 0, y: 0 };
        let sumX = 0;
        let sumY = 0;
        points.forEach(point => {
            sumX += Number.isFinite(Number(point && point.x)) ? Number(point.x) : 0;
            sumY += Number.isFinite(Number(point && point.y)) ? Number(point.y) : 0;
        });
        return { x: sumX / 4, y: sumY / 4 };
    }

    function getLinkedSlavePostRadius(link) {
        const raw = Number(link && link.postRadius);
        return Number.isFinite(raw) ? clamp(raw, 0, 1) : 0.25;
    }

    function pointInConvexQuad(points, px, py, epsilon = 1e-10) {
        if(!Array.isArray(points) || points.length !== 4) return false;
        const cross2D = (ax, ay, bx, by) => ax * by - ay * bx;
        let hasPositive = false;
        let hasNegative = false;
        for(let i = 0; i < 4; i++) {
            const j = (i + 1) % 4;
            const ex = points[j].x - points[i].x;
            const ey = points[j].y - points[i].y;
            const rx = px - points[i].x;
            const ry = py - points[i].y;
            const cross = cross2D(ex, ey, rx, ry);
            if(cross > epsilon) hasPositive = true;
            else if(cross < -epsilon) hasNegative = true;
            if(hasPositive && hasNegative) return false;
        }
        return true;
    }

    function invertBilinearQuad(points, px, py, maxIterations = 8) {
        if(!Array.isArray(points) || points.length !== 4) return { u: 0.5, v: 0.5 };
        const p00 = points[0];
        const p10 = points[1];
        const p11 = points[2];
        const p01 = points[3];
        let u = 0.5;
        let v = 0.5;
        for(let iter = 0; iter < maxIterations; iter++) {
            const fx = (1 - u) * (1 - v) * p00.x + u * (1 - v) * p10.x + u * v * p11.x + (1 - u) * v * p01.x - px;
            const fy = (1 - u) * (1 - v) * p00.y + u * (1 - v) * p10.y + u * v * p11.y + (1 - u) * v * p01.y - py;
            const j00 = (1 - v) * (p10.x - p00.x) + v * (p11.x - p01.x);
            const j01 = (1 - u) * (p01.x - p00.x) + u * (p11.x - p10.x);
            const j10 = (1 - v) * (p10.y - p00.y) + v * (p11.y - p01.y);
            const j11 = (1 - u) * (p01.y - p00.y) + u * (p11.y - p10.y);
            const det = j00 * j11 - j01 * j10;
            if(Math.abs(det) < 1e-12) break;
            u -= (j11 * fx - j01 * fy) / det;
            v -= (j00 * fy - j10 * fx) / det;
            if(u < -0.5 || u > 1.5 || v < -0.5 || v > 1.5) break;
        }
        return {
            u: clamp(u, 0, 1),
            v: clamp(v, 0, 1)
        };
    }

    function updatePreviewLinkedPostState(link, inside, srcX, srcY, postInfo) {
        if(!link || link.postEnabled !== true || !postInfo) return true;
        if(!inside) {
            // 曾激活后离开：报告“曾激活”，让上层持续执行归零回弹；同时清除当前激活态
            const wasActive = link._previewPostEverActive === true;
            link._previewPostActive = false;
            return wasActive;
        }
        if(link._previewPostActive !== true) {
            const dx = srcX - postInfo.posX;
            const dy = srcY - postInfo.posY;
            if(dx * dx + dy * dy <= postInfo.radius * postInfo.radius) {
                link._previewPostActive = true;
                link._previewPostEverActive = true;
            }
        }
        return link._previewPostActive === true;
    }

    function computeDirectionalLinkedPost(link, srcComponent, targetComponent) {
        if(!link || link.postEnabled !== true || !srcComponent || !targetComponent) return null;
        if(srcComponent.type !== 'joystick' || targetComponent.type !== 'joystick') return null;
        
        // 摇杆→摇杆 四边形区域: 碰撞桩 = 4 点几何中心（所有摇杆类型通用）
        if(getLinkedSlaveEffectiveRegionMode(link, srcComponent, targetComponent) === 'rect') {
            const pts = getLinkedSlaveRegionPoints(link);
            const center = getLinkedSlaveRegionCenter(pts);
            return {
                targetId: link.targetId,
                posX: center.x,
                posY: center.y,
                radius: getLinkedSlavePostRadius(link)
            };
        }
        
        // 方向摇杆→方向摇杆: 原有 perDirection 逻辑
        if(srcComponent.paramMode !== '4' || targetComponent.paramMode !== '4') return null;
        if(!Array.isArray(link.perDirection) || link.perDirection.length === 0) return null;

        const srcCfg = getJoystickConfig(srcComponent);
        const tgtCfg = getJoystickConfig(targetComponent);
        const dirs = Math.min(srcCfg.directionCount, tgtCfg.directionCount, link.perDirection.length);
        if(dirs <= 0) return null;

        let minBX = 0, maxBX = 0, minBY = 0, maxBY = 0;
        const dirInfos = [];
        for(let d = 0; d < dirs; d++) {
            const dirCfg = link.perDirection[d] || { srcMin: 0, srcMax: 0.5 };
            const sMin = Number.isFinite(Number(dirCfg.srcMin)) ? Number(dirCfg.srcMin) : 0;
            const sMax = Number.isFinite(Number(dirCfg.srcMax)) ? Number(dirCfg.srcMax) : 0.5;
            const angle = getJoystickDirectionAngle(srcComponent, d);
            const angleRad = angle * Math.PI / 180;
            const sinA = Math.sin(angleRad);
            const cosA = Math.cos(angleRad);
            dirInfos.push({ sMin, sMax, sinA, cosA });
            if(sMax > 0.001) {
                const range = sMax - sMin;
                const contribX = range * sinA;
                const contribY = range * cosA;
                if(contribX > 0) maxBX += contribX; else minBX += contribX;
                if(contribY > 0) maxBY += contribY; else minBY += contribY;
            }
        }

        const centerAccX = (minBX + maxBX) / 2;
        const centerAccY = (minBY + maxBY) / 2;
        let Wxx = 0, Wxy = 0, Wyx = 0, Wyy = 0;
        for(const info of dirInfos) {
            if(info.sMax <= 0.001) continue;
            const w = 1 / info.sMax;
            Wxx += info.sinA * info.sinA * w;
            Wxy += info.sinA * info.cosA * w;
            Wyx += info.cosA * info.sinA * w;
            Wyy += info.cosA * info.cosA * w;
        }
        const det = Wxx * Wyy - Wxy * Wyx;
        let posX;
        let posY;
        if(Math.abs(det) < 1e-10) {
            posX = centerAccX >= 0 ? 0.3 : -0.3;
            posY = centerAccY >= 0 ? 0.3 : -0.3;
        } else {
            posX = (Wyy * centerAccX - Wxy * centerAccY) / det;
            posY = (-Wyx * centerAccX + Wxx * centerAccY) / det;
        }

        return {
            targetId: link.targetId,
            posX,
            posY,
            radius: getLinkedSlavePostRadius(link)
        };
    }

    function getJoystickCollisionPosts(component) {
        if(!component || component.type !== 'joystick' || !Array.isArray(component.linkedSlaves)) return [];
        const posts = [];
        component.linkedSlaves.forEach((link) => {
            if(!link || !link.enabled || !link.targetId || link.postEnabled !== true) return;
            const targetComp = components.find(c => c.id === link.targetId);
            const post = computeDirectionalLinkedPost(link, component, targetComp);
            if(post) posts.push(post);
        });
        return posts;
    }

    /**
     * 收集摇杆所有联动四边形区域数据，用于工作区可视化渲染
     * @param {Object} component - 摇杆组件
     * @returns {Array} 区域数组，每个包含 points(4点), postCenter, postRadius, postEnabled
     */
    function getJoystickLinkedRegions(component) {
        if(!component || component.type !== 'joystick' || !Array.isArray(component.linkedSlaves)) return [];
        const regions = [];
        component.linkedSlaves.forEach((link) => {
            if(!link || !link.enabled || !link.targetId) return;
            const targetComp = components.find(c => c.id === link.targetId);
            if(!targetComp || targetComp.type !== 'joystick') return;
            if(getLinkedSlaveEffectiveRegionMode(link, component, targetComp) !== 'rect') return;

            const pts = getLinkedSlaveRegionPoints(link);
            const postCenter = getLinkedSlaveRegionCenter(pts);
            const postRadius = getLinkedSlavePostRadius(link);
            const postEnabled = link.postEnabled === true;
            
            regions.push({
                targetId: link.targetId,
                points: pts.map(p => ({x: p.x, y: p.y})),
                postCenterX: postCenter.x,
                postCenterY: postCenter.y,
                postRadius,
                postEnabled
            });
        });
        return regions;
    }

    function getJoystickRangeTriggerRegions(component) {
        if(!component || component.type !== 'joystick' || !Array.isArray(component.rangeTriggers)) return [];
        const regions = [];
        component.rangeTriggers.forEach((trigger, idx) => {
            if(!trigger) return;
            const pts = getRangeTriggerRegionPoints(trigger);
            regions.push({
                triggerIndex: idx,
                points: pts.map(p => ({ x: p.x, y: p.y }))
            });
        });
        return regions;
    }

    // 嵌套联动预览平滑值缓存（用于产生回弹效果）
    let linkedSmoothValues = new WeakMap();
    let previewActionVarValues = new Map();
    let previewRangeActionsInitialized = false;

    function getPreviewTextVisible(component) {
        if(dialogueRuntime && dialogueRuntime.textStates.has(component && component.id)) return dialogueRuntime.textStates.get(component.id).visible === true;
        if(!component || component.type !== 'text' || !isTextVisibilityEnabled(component)) return true;
        const variable = getTextVisibilityVar(component);
        if(previewTextVariableStates.has(variable)) return previewTextVariableStates.get(variable) === true;
        return component.visDefault !== false;
    }

    function applyPreviewTextClickBinding(component) {
        const variable = getTextClickVar(component);
        if(!variable) return false;
        let current;
        if(previewTextVariableStates.has(variable)) {
            current = previewTextVariableStates.get(variable) === true;
        } else {
            const displayOwner = components.find(item => item && isTextVisibilityEnabled(item) && getTextVisibilityVar(item) === variable);
            current = !!(displayOwner && displayOwner.visDefault !== false);
        }
        previewTextVariableStates.set(variable, !current);
        return true;
    }

    function composePreviewMappedJoystickValue(component, mappedValue, fallbackValue = null) {
        const base = fallbackValue && typeof fallbackValue === 'object'
            ? fallbackValue
            : (component && component.type === 'joystick' ? getJoystickPreviewBaseVector(component) : { x: 0, y: 0 });
        if(!mappedValue || typeof mappedValue !== 'object') return { x: base.x, y: base.y };

        let x = typeof mappedValue.x === 'number' ? mappedValue.x : null;
        let y = typeof mappedValue.y === 'number' ? mappedValue.y : null;
        if((x === null || y === null) && component && component.type === 'joystick' && component.paramMode === '4') {
            const cfg = getJoystickConfig(component);
            let accX = 0;
            let accY = 0;
            for(let d = 0; d < cfg.directionCount; d++) {
                const pressure = getJoystickDirectionPressureValue(component, mappedValue, d);
                if(pressure > 0) {
                    const anchor = getJoystickDirectionAnchorVector(component, d);
                    accX += pressure * anchor.x;
                    accY += pressure * anchor.y;
                }
            }
            if(x === null) x = accX;
            if(y === null) y = accY;
        }
        return {
            x: clamp(x === null ? base.x : x, -1, 1),
            y: clamp(y === null ? base.y : y, -1, 1)
        };
    }

    function getPreviewEffectiveSourceValue(component, linkedValues) {
        const baseValue = getComponentPreviewRawValue(component);
        if(!component || !linkedValues || !linkedValues.has(component.id)) return baseValue;
        const overrideValue = linkedValues.get(component.id);
        if(component.type === 'joystick') {
            return composePreviewMappedJoystickValue(component, overrideValue, baseValue);
        }
        return typeof overrideValue === 'number' ? overrideValue : baseValue;
    }

    function getPreviewLinkedScalarSourceValue(link, srcComponent, srcValue) {
        const srcIsDir = srcComponent && srcComponent.type === 'joystick' && srcComponent.paramMode === '4';
        const axis = (link && link.joyAxis !== undefined && link.joyAxis !== null && link.joyAxis !== '') ? link.joyAxis : 'both';
        if(srcIsDir && axis !== 'x' && axis !== 'y' && axis !== 'both') {
            const dirIdx = parseInt(axis, 10);
            if(Number.isFinite(dirIdx)) {
                const dirVals = getPreviewJoystickDirectionValues(srcComponent, srcValue);
                return dirVals[dirIdx] ?? 0;
            }
            return Math.max(Math.abs(srcValue.x || 0), Math.abs(srcValue.y || 0));
        }
        if(srcValue && typeof srcValue === 'object') {
            if(axis === 'x') return srcValue.x;
            if(axis === 'y') return srcValue.y;
            return Math.max(Math.abs(srcValue.x || 0), Math.abs(srcValue.y || 0));
        }
        return typeof srcValue === 'number' ? srcValue : 0;
    }

    function applyPreviewConfiguredActions(actions) {
        actions.forEach(action => {
            if(!action || !action.var) return;
            previewActionVarValues.set(action.var, Number.isFinite(Number(action.value)) ? Number(action.value) : 0);
        });
    }

    function getPreviewDefaultSourceValue(component) {
        if(!component) return 0;
        if(component.type === 'joystick') return getJoystickPreviewBaseVector(component);
        if(component.type && component.type.includes('slider')) return getSliderPreviewBaseValue(component);
        return 0;
    }

    function computePreviewRangeActive(kind, cfg, srcComp, targetComp, srcValue) {
        if(!srcComp || !srcValue) return false;
        let active = false;
        if(srcComp.type === 'joystick' && srcValue && typeof srcValue === 'object') {
            if(kind === 'trigger') {
                active = pointInConvexQuad(getRangeTriggerRegionPoints(cfg), srcValue.x, srcValue.y);
            } else if(getLinkedSlaveEffectiveRegionMode(cfg, srcComp, targetComp) === 'rect') {
                active = pointInConvexQuad(getLinkedSlaveRegionPoints(cfg), srcValue.x, srcValue.y);
            } else {
                const scalar = getPreviewLinkedScalarSourceValue(cfg, srcComp, srcValue);
                const sMin = cfg.srcMin ?? 0;
                const sMax = cfg.srcMax ?? 0.5;
                active = scalar >= sMin && scalar <= sMax;
            }
        } else {
            const scalar = typeof srcValue === 'number' ? srcValue : 0;
            const sMin = cfg.srcMin ?? 0;
            const sMax = cfg.srcMax ?? 0.5;
            active = scalar >= sMin && scalar <= sMax;
        }
        return active;
    }

    function updatePreviewNestedActionStates(linkedValues) {
        if(!previewRangeActionsInitialized) {
            previewRangeActionsInitialized = true;
            const initEntries = [];
            components.forEach(srcComp => {
                if(!srcComp) return;
                const defVal = getPreviewDefaultSourceValue(srcComp);
                if(Array.isArray(srcComp.linkedSlaves)) {
                    srcComp.linkedSlaves.forEach(link => {
                        if(!link || !link.enabled) return;
                        const enterActions = getLinkedSlaveConfiguredActions(link, 'enter');
                        const leaveActions = getLinkedSlaveConfiguredActions(link, 'leave');
                        if(enterActions.length === 0 && leaveActions.length === 0) return;
                        const targetComp = link.targetId ? components.find(c => c.id === link.targetId) : null;
                        initEntries.push({ holder: link, key: '_previewRangeState', active: computePreviewRangeActive('link', link, srcComp, targetComp, defVal), enterActions, leaveActions });
                    });
                }
                if(Array.isArray(srcComp.rangeTriggers)) {
                    srcComp.rangeTriggers.forEach(trigger => {
                        if(!trigger) return;
                        const enterActions = normalizeLinkedSlaveActionList(trigger.enterActions).filter(a => a && a.var);
                        const leaveActions = normalizeLinkedSlaveActionList(trigger.leaveActions).filter(a => a && a.var);
                        if(enterActions.length === 0 && leaveActions.length === 0) return;
                        initEntries.push({ holder: trigger, key: '_previewRangeState', active: computePreviewRangeActive('trigger', trigger, srcComp, null, defVal), enterActions, leaveActions });
                    });
                }
            });
            // 初始状态：离开优先、进入最后，避免共享动作变量互相覆盖
            initEntries.filter(e => !e.active).forEach(e => {
                e.holder[e.key] = false;
                applyPreviewConfiguredActions(e.leaveActions);
            });
            initEntries.filter(e => e.active).forEach(e => {
                e.holder[e.key] = true;
                applyPreviewConfiguredActions(e.enterActions);
            });
        }
        // 每帧边沿触发，两阶段应用（先处理所有“离开”，再处理所有“进入”），
        // 与生成端一致，避免多个区间共享同一动作变量时互相覆盖。
        const transitions = [];
        components.forEach(srcComp => {
            if(!srcComp) return;
            const srcValue = getPreviewEffectiveSourceValue(srcComp, linkedValues);
            if(Array.isArray(srcComp.linkedSlaves)) {
                srcComp.linkedSlaves.forEach(link => {
                    if(!link || !link.enabled) return;
                    const enterActions = getLinkedSlaveConfiguredActions(link, 'enter');
                    const leaveActions = getLinkedSlaveConfiguredActions(link, 'leave');
                    if(enterActions.length === 0 && leaveActions.length === 0) return;
                    const targetComp = link.targetId ? components.find(c => c.id === link.targetId) : null;
                    transitions.push({ holder: link, key: '_previewRangeState', active: computePreviewRangeActive('link', link, srcComp, targetComp, srcValue), enterActions, leaveActions });
                });
            }
            if(Array.isArray(srcComp.rangeTriggers)) {
                srcComp.rangeTriggers.forEach(trigger => {
                    if(!trigger) return;
                    const enterActions = normalizeLinkedSlaveActionList(trigger.enterActions).filter(a => a && a.var);
                    const leaveActions = normalizeLinkedSlaveActionList(trigger.leaveActions).filter(a => a && a.var);
                    if(enterActions.length === 0 && leaveActions.length === 0) return;
                    transitions.push({ holder: trigger, key: '_previewRangeState', active: computePreviewRangeActive('trigger', trigger, srcComp, null, srcValue), enterActions, leaveActions });
                });
            }
        });
        // 离开优先（旧状态为 inside → 现在 outside）
        transitions.filter(t => !t.active && !!t.holder[t.key]).forEach(t => {
            applyPreviewConfiguredActions(t.leaveActions);
        });
        // 进入最后（旧状态为 outside → 现在 inside），确保共享变量以“进入”为准
        transitions.filter(t => t.active && !t.holder[t.key]).forEach(t => {
            applyPreviewConfiguredActions(t.enterActions);
        });
        // 统一更新状态
        transitions.forEach(t => {
            t.holder[t.key] = t.active;
        });
    }

    function applyPreviewDependencyTriggerValues(vars, snapshot) {
        components.forEach((component, compIdx) => {
            if(!component) return;
            const specs = getComponentDependencyTriggerSpecs(component, compIdx);
            if(specs.length === 0) return;

            const current = getPreviewComponentSignal(component, snapshot);
            const joyDirValues = component.type === 'joystick' && component.paramMode === '4'
                ? getPreviewJoystickDirectionValues(component, current)
                : null;
            const sliderSegmentValues = isSliderSubdivisionMode(component)
                ? getSliderSubdivisionValues(component, current)
                : null;

            specs.forEach(spec => {
                let active = false;
                if(spec.triggerKind === 'bidir') {
                    active = spec.directions.some(dir => dir === 0 ? current < 0.5 : current > 0.5);
                } else if(spec.triggerKind === 'grid') {
                    if(spec.gridIndex === 0) active = current < spec.thresholdHigh;
                    else if(spec.gridIndex === spec.gridSteps - 1) active = current >= spec.thresholdLow;
                    else active = current >= spec.thresholdLow && current < spec.thresholdHigh;
                } else if(spec.triggerKind === 'joy-dir') {
                    active = !!(joyDirValues && (joyDirValues[spec.paramIndex] ?? 0) >= 0.001);
                } else if(spec.triggerKind === 'joy-axis') {
                    const axisVal = spec.paramIndex === 0 ? (current.x || 0) : (current.y || 0);
                    active = axisVal >= 0.001;
                } else if(spec.triggerKind === 'slider-segment') {
                    active = !!(sliderSegmentValues && (sliderSegmentValues[spec.paramIndex] ?? 0) >= 0.001);
                } else {
                    active = current >= 0.001;
                }

                const prevActive = vars.get(spec.stateVar) === 1;
                if(active && !prevActive) {
                    vars.set(spec.targetVar, spec.trueValue);
                    vars.set(spec.stateVar, 1);
                } else if(!active && prevActive) {
                    if(spec.useElse) vars.set(spec.targetVar, spec.falseValue);
                    vars.set(spec.stateVar, 0);
                } else if(!prevActive && !vars.has(spec.stateVar)) {
                    vars.set(spec.stateVar, 0);
                }
            });
        });
    }

    function getLinkedSmoothedValue(component, targetValue, axis) {
        if(!component || typeof targetValue !== 'number') return targetValue;
        const key = axis || 'val';
        let entry = linkedSmoothValues.get(component);
        if(!entry) {
            entry = {};
            linkedSmoothValues.set(component, entry);
        }
        if(!entry[key]) {
            entry[key] = { value: targetValue, vel: 0 };
        }
        const state = entry[key];
        // 弹簧动力学：产生自然回弹
        const springK = 0.08;  // 弹性
        const springD = 0.88;  // 阻尼
        const force = (targetValue - state.value) * springK;
        state.vel = (state.vel + force) * springD;
        state.value += state.vel;
        // 接近目标时直接收敛
        if(Math.abs(state.value - targetValue) < 0.001 && Math.abs(state.vel) < 0.0001) {
            state.value = targetValue;
            state.vel = 0;
        }
        return state.value;
    }

    /**
     * 计算所有嵌套联动的目标值。
     * 遍历所有组件的 linkedSlaves，根据源/目标类型、区间配置，
     * 计算出每个被联动组件的覆写值，用于工作区预览。
     * @returns {Map<string, any>} 目标组件ID → 覆写值 (滑块存数字, XY摇杆存 {x,y}, 方向摇杆存 {方向索引:值, x, y})
     */
    function computeAllLinkedValues() {
        if(linkedValuesCache) return linkedValuesCache;  // 缓存命中则直接返回
        
        const linkedValues = new Map();
        components.forEach(src => {                       // ---- 第一轮: 遍历所有源组件 ----
            if(!src || !Array.isArray(src.linkedSlaves)) return;
            const srcValue = getComponentPreviewRawValue(src);  // 滑块:数字, 摇杆:{x,y}
            src.linkedSlaves.forEach(link => {
                if(!link || !link.enabled || !link.targetId) return;
                const targetComp = components.find(c => c.id === link.targetId);
                if(!targetComp) return;
                
                if(src.type === 'joystick') {
                    // ========== 路径 A: 摇杆源 ==========
                    const srcIsDir = src.paramMode === '4';      // 源是否方向模式
                    const tgtIsDir = targetComp.type === 'joystick' && targetComp.paramMode === '4';  // 目标是否方向模式
                    
                    const effectiveRegionMode = getLinkedSlaveEffectiveRegionMode(link, src, targetComp);

                        {
                        // A2: XY摇杆 → 四边形区域映射
                        const isRectMode = effectiveRegionMode === 'rect';
                        if(isRectMode && typeof srcValue === 'object') {
                            const pts = getLinkedSlaveRegionPoints(link);
                            const sx = srcValue.x, sy = srcValue.y;
                            const overflow = link.overflow || 'reset';
                            const inside = pointInConvexQuad(pts, sx, sy);
                            const postEntry = computeDirectionalLinkedPost(link, src, targetComp);
                            const postActivated = updatePreviewLinkedPostState(link, inside, sx, sy, postEntry);
                            if(postEntry) src._linkedPostEnabled = true;

                            // 碰撞桩未激活（尚未经过碰撞桩）：不产生任何联动绑定，
                            // 不写覆盖值、不归零，目标组件保持自由（可独立拖拽/自动动画正常播放）
                            if(link.postEnabled === true && !postActivated) {
                                // no-op
                            } else if(!inside && overflow === 'reset') {
                                if(targetComp && targetComp.type === 'joystick') {
                                    const existing = linkedValues.get(link.targetId) || {};
                                    existing.x = 0;
                                    existing.y = 0;
                                    linkedValues.set(link.targetId, existing);
                                } else if(targetComp) {
                                    linkedValues.set(link.targetId, targetComp.paramMode === '2' ? 0.5 : 0);
                                }
                            } else if(postActivated || link.postEnabled !== true) {
                                const uv = invertBilinearQuad(pts, sx, sy);
                                const u = uv.u;
                                const v = uv.v;

                                if(targetComp && targetComp.type === 'joystick') {
                                    // 摇杆→摇杆: 始终双轴映射, (u,v)→[-1,1]²
                                    const tgtX = u * 2 - 1;
                                    const tgtY = v * 2 - 1;
                                    const existing = linkedValues.get(link.targetId) || {};
                                    existing.x = clamp(tgtX, -1, 1);
                                    existing.y = clamp(1 - v * 2, -1, 1);
                                    link._previewLastMappedValue = { x: existing.x, y: existing.y };
                                    linkedValues.set(link.targetId, existing);
                                } else if(targetComp) {
                                    // 目标为滑块: 使用 u 方向归一化值
                                    const mappedVal = u;
                                    let result = clamp(mappedVal, 0, 1);
                                    if(isSliderGridMode(targetComp)) {
                                        const gridSteps = Math.max(2, targetComp.gridSteps || 3);
                                        const gridIdx = clamp(Math.round(result * (gridSteps - 1)), 0, gridSteps - 1);
                                        result = gridIdx / (gridSteps - 1);
                                    }
                                    if(targetComp.paramMode === '2') {
                                        const side = link.splitSide || 'both';
                                        if(side === 'left') result = 0.5 - result * 0.5;
                                        else if(side === 'right') result = 0.5 + result * 0.5;
                                    }
                                    link._previewLastMappedValue = result;
                                    linkedValues.set(link.targetId, result);
                                }
                            } else if(overflow === 'keep_max' && link._previewLastMappedValue !== undefined) {
                                const last = link._previewLastMappedValue;
                                if(targetComp && targetComp.type === 'joystick' && last && typeof last === 'object') {
                                    const existing = linkedValues.get(link.targetId) || {};
                                    existing.x = clamp(Number(last.x) || 0, -1, 1);
                                    existing.y = clamp(Number(last.y) || 0, -1, 1);
                                    linkedValues.set(link.targetId, existing);
                                } else if(targetComp && typeof last === 'number') {
                                    linkedValues.set(link.targetId, clamp(last, 0, 1));
                                }
                            }
                        } else {
                        // A3: 摇杆源 → 单轴 / 矩形区域外的默认路径
                        // 从摇杆中提取标量源值: XY摇杆选X/Y/two方向, 方向摇杆选方向索引
                        const axis = (link.joyAxis !== undefined && link.joyAxis !== null && link.joyAxis !== '') ? link.joyAxis : 'both';
                        let srcVal;  // 提取的标量源值
                        if(srcIsDir && axis !== 'x' && axis !== 'y' && axis !== 'both') {
                            // 方向源→非方向目标: 取该方向的压感值 [0,1]
                            const dirIdx = parseInt(axis, 10);
                            if(Number.isFinite(dirIdx)) {
                                const dirVals = getPreviewJoystickDirectionValues(src, srcValue);
                                srcVal = dirVals[dirIdx] ?? 0;
                            } else {
                                srcVal = Math.max(Math.abs(srcValue.x), Math.abs(srcValue.y));
                            }
                        } else if(typeof srcValue === 'object') {
                            // XY 摇杆按轴取值: X 轴 [-1,1], Y 轴 [-1,1], both 取两轴最大绝对值
                            if(axis === 'x') srcVal = srcValue.x;
                            else if(axis === 'y') srcVal = srcValue.y;
                            else srcVal = Math.max(Math.abs(srcValue.x), Math.abs(srcValue.y));
                        } else {
                            srcVal = srcValue;
                        }
                        
                        if(targetComp.type === 'joystick') {
                        // A3a: 摇杆→摇杆 → 始终映射到目标摇杆双轴
                        const tgtAxis = (link.joyTargetAxis !== undefined && link.joyTargetAxis !== null && link.joyTargetAxis !== '') ? link.joyTargetAxis : 'both';
                        const isTgtDir = targetComp.paramMode === '4';
                        const srcMin = link.srcMin ?? 0;
                        const srcMax = link.srcMax ?? 0.5;
                        const range = srcMax - srcMin;
                        const overflow = link.overflow || 'reset';
                        const splitSide = link.splitSide || 'both';
                        let finalVal;
                        
                        // 区间内: srcVal 归一化 → 根据目标类型转换为目标值域
                        if(range > 0 && srcVal >= srcMin && srcVal <= srcMax) {
                            let normVal = clamp((srcVal - srcMin) / range, 0, 1);  // [0,1]
                            // 目标为格子模式: 吸附到格子步进
                            if(isSliderGridMode(targetComp)) {
                                const gs = Math.max(2, targetComp.gridSteps || 3);
                                normVal = clamp(Math.round(normVal * (gs - 1)), 0, gs - 1) / (gs - 1);
                            }
                            // 目标为双向模式: 分边映射
                            if(targetComp.paramMode === '2') {
                                if(splitSide === 'left') normVal = 0.5 - normVal * 0.5;
                                else if(splitSide === 'right') normVal = 0.5 + normVal * 0.5;
                            }
                            // 根据目标类型转换为目标值域
                            if(isTgtDir) {
                                finalVal = normVal;                // 方向摇杆: 方向压感 [0,1]
                            } else if(splitSide === 'left') {
                                finalVal = normVal - 1;            // XY摇杆左半: [-1,0]
                            } else if(splitSide === 'right') {
                                finalVal = normVal;                // XY摇杆右半: [0,1]
                            } else {
                                finalVal = normVal * 2 - 1;        // XY摇杆全轴: [-1,1]
                            }
                        } else if(overflow === 'reset') {
                            finalVal = 0;  // 溢出归零
                        } else { // keep_max: 低于区间→最小值, 高于区间→最大值
                            if(srcVal < srcMin) {
                                finalVal = isTgtDir ? 0 : (splitSide === 'left' ? -1 : (splitSide === 'right' ? 0 : -1));
                            } else {
                                finalVal = isTgtDir ? 1 : (splitSide === 'left' ? 0 : (splitSide === 'right' ? 1 : 1));
                            }
                        }
                        const existing = linkedValues.get(link.targetId) || {};
                        // 摇杆→摇杆: 非方向目标写双轴, 方向目标按方向索引
                        if(isTgtDir) {
                            existing[tgtAxis] = finalVal;
                        } else {
                            existing.x = finalVal;
                            existing.y = finalVal;
                        }
                        linkedValues.set(link.targetId, existing);
                    } else {
                        // A3b: 摇杆→滑条 → 将摇杆轴的标量值映射到滑块 [0,1]
                        // 双向滑条目标时可选"右半轴" (joyAxis2), 双轴分半合成滑条位置
                        const isTgtBidir = targetComp.paramMode === '2';
                        const hasRightAxis = isTgtBidir && link.joyAxis2 && link.joyAxis2 !== '';
                        if(hasRightAxis && typeof srcValue === 'object') {
                            // 摇杆→双向滑条 (双轴): 提取右半轴值
                            let srcVal2;
                            const axis2 = link.joyAxis2;
                            if(srcIsDir && axis2 !== 'x' && axis2 !== 'y') {
                                // 方向源→双向滑条: 按方向索引取压感值
                                const dirIdx2 = parseInt(axis2, 10);
                                if(Number.isFinite(dirIdx2)) {
                                    const dirVals = getPreviewJoystickDirectionValues(src, srcValue);
                                    srcVal2 = dirVals[dirIdx2] ?? 0;
                                } else {
                                    srcVal2 = 0;
                                }
                            } else if(axis2 === 'x') {
                                srcVal2 = srcValue.x;
                            } else if(axis2 === 'y') {
                                srcVal2 = srcValue.y;
                            } else {
                                srcVal2 = 0;
                            }
                            // 左半轴归一化 [0,1], 右半轴归一化 [0,1]
                            const bidirSrcMin = link.srcMin ?? 0;
                            const bidirSrcMax = link.srcMax ?? 0.5;
                            const bidirRange = bidirSrcMax - bidirSrcMin;
                            const leftNorm = bidirRange > 0 ? clamp((srcVal - bidirSrcMin) / bidirRange, 0, 1) : 0;
                            const rightNorm = bidirRange > 0 ? clamp((srcVal2 - bidirSrcMin) / bidirRange, 0, 1) : 0;
                            // 双向滑条合成: 滑条值 = 0.5 + (right - left) * 0.5
                            // 左半压感→滑条 0~0.5, 右半压感→滑条 0.5~1, 同压感→中心 0.5
                            const combinedVal = 0.5 + (rightNorm - leftNorm) * 0.5;
                            linkedValues.set(link.targetId, clamp(combinedVal, 0, 1));
                        } else {
                            // 摇杆→普通滑条: 标量源值直接映射 [0,1]
                            const mappedVal = computeLinkedMappedValue(
                                srcVal, link.srcMin, link.srcMax, 
                                link.overflow || 'reset', link.splitSide || 'both', targetComp
                            );
                            linkedValues.set(link.targetId, mappedVal);
                        }
                    }
                    }
                    } // A3 结束: 摇杆源单轴
                } else {
                    // ========== 路径 B: 滑块源 ==========
                    const srcVal = typeof srcValue === 'number' ? srcValue : 0;  // 滑块值始终为数字 [0,1]
                    if(targetComp.type === 'joystick') {
                        // B1: 滑块→摇杆 → 将滑块值映射到目标摇杆的指定轴/方向
                        const tgtAxis = (link.joyTargetAxis !== undefined && link.joyTargetAxis !== null && link.joyTargetAxis !== '') ? link.joyTargetAxis : 'x';
                        const isTgtDir = targetComp.paramMode === '4';
                        const srcMin = link.srcMin ?? 0;
                        const srcMax = link.srcMax ?? 0.5;
                        const range = srcMax - srcMin;
                        const overflow = link.overflow || 'reset';
                        const splitSide = link.splitSide || 'both';
                        let finalVal;
                        
                        if(range > 0 && srcValue >= srcMin && srcValue <= srcMax) {
                            // 区间内：归一化 [0,1]
                            let normVal = clamp((srcValue - srcMin) / range, 0, 1);
                            // 格子模式吸附
                            if(isSliderGridMode(targetComp)) {
                                const gs = Math.max(2, targetComp.gridSteps || 3);
                                normVal = clamp(Math.round(normVal * (gs - 1)), 0, gs - 1) / (gs - 1);
                            }
                            // 双向模式分边
                            if(targetComp.paramMode === '2') {
                                if(splitSide === 'left') normVal = 0.5 - normVal * 0.5;
                                else if(splitSide === 'right') normVal = 0.5 + normVal * 0.5;
                            }
                            // 转换为目标摇杆轴值
                            if(isTgtDir) {
                                finalVal = normVal; // 方向值 [0,1]
                            } else if(splitSide === 'left') {
                                finalVal = normVal - 1;   // [0,1] → [-1,0]
                            } else if(splitSide === 'right') {
                                finalVal = normVal;       // [0,1] → [0,1]
                            } else {
                                finalVal = normVal * 2 - 1; // [0,1] → [-1,1]
                            }
                        } else if(overflow === 'reset') {
                            // 超出归零：摇杆回中心
                            finalVal = isTgtDir ? 0 : 0;
                        } else { // keep_max
                            if(srcValue < srcMin) {
                                // 低于区间：保持最小值
                                finalVal = isTgtDir ? 0 : (splitSide === 'left' ? -1 : (splitSide === 'right' ? 0 : -1));
                            } else {
                                // 高于区间：保持最大值
                                finalVal = isTgtDir ? 1 : (splitSide === 'left' ? 0 : (splitSide === 'right' ? 1 : 1));
                            }
                        }
                        const existing = linkedValues.get(link.targetId) || {};
                        // B2: 双向滑条→方向摇杆 → 左半压感映射到方向1, 右半压感映射到方向2
                        const isBidirToDir = isTgtDir && src.paramMode === '2' && link.joyTargetAxis2 !== null && link.joyTargetAxis2 !== undefined && link.joyTargetAxis2 !== '';
                        if(isBidirToDir && range > 0 && srcValue >= srcMin && srcValue <= srcMax) {
                            // 区间内: 滑条值 0.5 为分界, 左侧 → 方向1 压感, 右侧 → 方向2 压感
                            const leftVal = clamp((0.5 - srcValue) / 0.5, 0, 1);   // 0.5→0.0 映射到压感 1→0
                            const rightVal = clamp((srcValue - 0.5) / 0.5, 0, 1);  // 0.5→1.0 映射到压感 0→1
                            existing[tgtAxis] = leftVal;
                            const dir2 = parseInt(link.joyTargetAxis2, 10);
                            if(Number.isFinite(dir2)) existing[dir2] = rightVal;
                        } else if(isBidirToDir) {
                            // 溢出处理
                            if(overflow === 'reset') {
                                existing[tgtAxis] = 0;
                                const dir2 = parseInt(link.joyTargetAxis2, 10);
                                if(Number.isFinite(dir2)) existing[dir2] = 0;
                            } else {  // keep_max
                                existing[tgtAxis] = srcValue < 0.5 ? 1 : 0;
                                const dir2 = parseInt(link.joyTargetAxis2, 10);
                                if(Number.isFinite(dir2)) existing[dir2] = srcValue > 0.5 ? 1 : 0;
                            }
                        } else {
                            existing[tgtAxis] = finalVal;
                        }
                        linkedValues.set(link.targetId, existing);
                    } else {
                        // B3: 滑块→滑块 → 直接通过 computeLinkedMappedValue 映射
                        const mappedVal = computeLinkedMappedValue(
                            srcValue, link.srcMin, link.srcMax, 
                            link.overflow || 'reset', link.splitSide || 'both', targetComp
                        );
                        linkedValues.set(link.targetId, mappedVal);
                    }
                }
            });
        });
        // ---- 后处理: 方向摇杆从逐方向值合成 XY 向量 ----
        // 多个滑块/摇杆可能各自写入不同方向 (eg, 方向0=0.5, 方向2=0.3),
        // 需要合成 XY 向量用于工作区预览渲染手柄位置
        linkedValues.forEach((val, targetId) => {
            const tgt = components.find(c => c.id === targetId);
            if(!tgt || tgt.type !== 'joystick' || tgt.paramMode !== '4') return;
            if(!val || typeof val !== 'object' || typeof val.x === 'number') return; // 已有 x/y 则无需合成
            const cfg = getJoystickConfig(tgt);
            let accX = 0, accY = 0;
            let hasAny = false;
            for(let d = 0; d < cfg.directionCount; d++) {
                const pressure = getJoystickDirectionPressureValue(tgt, val, d);
                if(pressure > 0) {
                    const anchor = getJoystickDirectionAnchorVector(tgt, d);
                    accX += pressure * anchor.x;
                    accY += pressure * anchor.y;
                    hasAny = true;
                }
            }
            if(hasAny) {
                val.x = clamp(accX, -1, 1);
                val.y = clamp(accY, -1, 1);
            }
        });
        linkedValuesCache = linkedValues;
        return linkedValues;
    }

    /** 获取组件的联动覆盖值（如有嵌套映射则返回映射后的值，否则返回 null） */
    function getLinkedOverrideValue(component) {
        if(!component) return null;
        const linkedValues = computeAllLinkedValues();
        return linkedValues.has(component.id) ? linkedValues.get(component.id) : null;
    }

    function sliderUsesExplicitRange(component) {
        return !!component && !!component.type && component.type.includes('slider') && component.paramMode === '1';
    }

    function getSliderRangeMin(component) {
        if(!sliderUsesExplicitRange(component)) return 0;
        return autoNumber(component.minVals && component.minVals[0], 0);
    }

    function getSliderRangeMax(component) {
        if(!component || !component.type || !component.type.includes('slider')) return 1;
        if(!sliderUsesExplicitRange(component)) return autoNumber(component.maxVals && component.maxVals[0], 1);
        const min = getSliderRangeMin(component);
        const rawMax = autoNumber(component.maxVals && component.maxVals[0], 1);
        return rawMax < min ? min : rawMax;
    }

    function getSliderRangeSpan(component) {
        return Math.max(getSliderRangeMax(component) - getSliderRangeMin(component), 0.000001);
    }

    function sliderNormalizedToActual(component, normalizedValue) {
        const normalized = clamp(Number(normalizedValue) || 0, 0, 1);
        if(!sliderUsesExplicitRange(component)) return normalized;
        return getSliderRangeMin(component) + normalized * (getSliderRangeMax(component) - getSliderRangeMin(component));
    }

    function sliderActualToNormalized(component, actualValue) {
        if(!sliderUsesExplicitRange(component)) return clamp(Number(actualValue) || 0, 0, 1);
        return clamp((autoNumber(actualValue, getSliderRangeMin(component)) - getSliderRangeMin(component)) / getSliderRangeSpan(component), 0, 1);
    }

    function clampSliderActualValue(component, actualValue) {
        if(!sliderUsesExplicitRange(component)) return clamp(Number(actualValue) || 0, 0, 1);
        const min = getSliderRangeMin(component);
        const max = getSliderRangeMax(component);
        return clamp(autoNumber(actualValue, min), min, max);
    }

    function getSliderStoredDefaultValue(component) {
        if(!component || !component.type || !component.type.includes('slider')) return 0;
        if(component.paramMode === '2') return clamp(Number((component.defVals && component.defVals[0]) ?? 0.5), 0, 1);
        if(component.paramMode === '3') return clamp(Math.round(Number((component.defVals && component.defVals[0]) ?? 0)), 0, Math.max(0, (component.gridSteps || 3) - 1));
        if(sliderUsesExplicitRange(component)) {
            return clampSliderActualValue(component, (component.defVals && component.defVals[0]) ?? getSliderRangeMin(component));
        }
        return clamp(Number((component.defVals && component.defVals[0]) ?? 0), 0, 1);
    }

    function getSliderPreviewBaseValue(component) {
        if(!component || !component.type || !component.type.includes('slider')) return 0;
        if(component.paramMode === '2') return clamp(Number((component.defVals && component.defVals[0]) ?? 0.5), 0, 1);
        if(component.paramMode === '3') return clamp(((Number((component.defVals && component.defVals[0]) ?? 0)) / Math.max(1, (component.gridSteps || 3) - 1)), 0, 1);
        return sliderActualToNormalized(component, getSliderStoredDefaultValue(component));
    }

    function getJoystickPreviewBaseVector(component) {
        if(!component || component.type !== 'joystick') return { x: 0, y: 0 };
        return {
            x: clamp(Number(component.joystickDefaultX ?? ((component.defVals && component.defVals[0]) ?? 0)), -1, 1),
            y: clamp(Number(component.joystickDefaultY ?? ((component.defVals && component.defVals[1]) ?? 0)), -1, 1)
        };
    }

    function buildPreviewInteractiveSignature(component) {
        return JSON.stringify({
            type: component.type,
            paramMode: component.paramMode,
            physics: !!component.physics,
            physicsProfile: component.physicsProfile || 'normal',
            autoAnimate: !!component.autoAnimate,
            autoSource: getAutoSourceMode(component),
            autoAmpX: autoNumber(component.autoAmpX, 1),
            autoAmpY: autoNumber(component.autoAmpY, 1),
            autoSeedX: autoNumber(component.autoSeedX, 0.3187),
            autoSeedY: autoNumber(component.autoSeedY, 0.6123),
            autoFuncX: component.autoFuncX || '',
            autoFuncY: component.autoFuncY || '',
            autoSpeed: autoNumber(component.autoSpeed, 0.015),
            autoResponse: autoNumber(component.autoResponse, 0.22),
            autoBounce: autoNumber(component.autoBounce, 0.25),
            autoStr: autoNumber(component.autoStr, 0.1),
            gravity: autoNumber(component.gravity, 0),
            chaosRate: autoNumber(component.chaosRate, 96),
            springK: autoNumber(component.springK, 0.05),
            springD: autoNumber(component.springD, 0.95),
            minVals: Array.isArray(component.minVals) ? component.minVals.slice(0, 2) : [],
            defVals: Array.isArray(component.defVals) ? component.defVals.slice(0, 2) : [],
            maxVals: Array.isArray(component.maxVals) ? component.maxVals.slice(0, 2) : [],
            gridSteps: component.gridSteps || 3,
            gridValueStart: getSliderGridValueStart(component),
            gridValueStep: getSliderGridValueStep(component),
            sliderSubdivisions: component.sliderSubdivisions || 1,
            joystickDefaultX: autoNumber(component.joystickDefaultX, 0),
            joystickDefaultY: autoNumber(component.joystickDefaultY, 0),
            joystickDirectionCount: component.joystickDirectionCount || 4,
            joystickSubdivisions: component.joystickSubdivisions || 1,
            joystickAngleOffset: autoNumber(component.joystickAngleOffset, 0)
        });
    }

    function readPreviewFunctionSample(samples, phase) {
        if(!Array.isArray(samples) || samples.length === 0) return 0;
        const count = samples.length;
        const idx = Math.min(count - 1, Math.max(0, Math.floor(wrap01(phase) * count)));
        return autoNumber(samples[idx], 0);
    }

    function initPreviewInteractiveState(component, nowSec) {
        const signature = buildPreviewInteractiveSignature(component);
        const baseSliderValue = getSliderPreviewBaseValue(component);
        const baseJoy = getJoystickPreviewBaseVector(component);
        const autoSource = getAutoSourceMode(component);
        const runtime = {
            signature,
            lastTime: nowSec,
            accumulator: 0,
            counter: Math.max(0, Math.round(autoNumber(component.chaosRate, 96)) - 1),
            autoPrev: 0,
            autoPhase: 0,
            chaosX: autoNumber(component.autoSeedX, 0.3187),
            chaosY: autoNumber(component.autoSeedY, 0.6123),
            samplesX: autoSource === 'function' ? buildAutoFunctionSamples(component, 'x') : null,
            samplesY: component.type === 'joystick' && autoSource === 'function' ? buildAutoFunctionSamples(component, 'y') : null
        };
        if(component.type === 'joystick') {
            runtime.valueX = baseJoy.x;
            runtime.valueY = baseJoy.y;
            runtime.velX = 0;
            runtime.velY = 0;
            runtime.autoGoalX = baseJoy.x;
            runtime.autoGoalY = baseJoy.y;
            runtime.autoTgtX = baseJoy.x;
            runtime.autoTgtY = baseJoy.y;
        } else {
            runtime.value = baseSliderValue;
            runtime.vel = 0;
            runtime.autoGoal = baseSliderValue;
            runtime.autoTgt = baseSliderValue;
        }
        return runtime;
    }

    function getPreviewInteractiveState(component, nowSec = getPreviewTimeSeconds()) {
        if(!component || !component.id || !componentSupportsPhysics(component) || !component.physics) return null;
        const signature = buildPreviewInteractiveSignature(component);
        let runtime = previewInteractiveRuntime.get(component.id);
        if(!runtime || runtime.signature !== signature) {
            runtime = initPreviewInteractiveState(component, nowSec);
            previewInteractiveRuntime.set(component.id, runtime);
            return runtime;
        }
        let delta = nowSec - runtime.lastTime;
        runtime.lastTime = nowSec;
        if(!Number.isFinite(delta) || delta <= 0) return runtime;
        delta = Math.min(delta, 0.1);
        runtime.accumulator += delta;
        const fixedStep = 1 / 60;
        let guard = 0;
        while(runtime.accumulator >= fixedStep && guard < 12) {
            stepPreviewInteractiveState(component, runtime);
            runtime.accumulator -= fixedStep;
            guard++;
        }
        return runtime;
    }

    function stepPreviewInteractiveState(component, runtime) {
        if(!component || !runtime) return;
        if(component.type === 'joystick') {
            stepPreviewJoystickState(component, runtime);
        } else if(component.type === 'slider_h' || component.type === 'slider_v') {
            stepPreviewSliderState(component, runtime);
        }
    }

    /**
     * 预览侧联动"占用闩锁"判定：任一启用映射（linkedSlaves 且指向该组件）当前覆盖该组件时返回 true。
     * 与导出端 $linked_active_<idx> 语义一致：联动激活期间物理自动驱动暂停，
     * 从而保证预览与导出对 $auto 的处理一致（A8 修复；联动离开后恢复自动取决于绑定者/默认值）。
     */
    function isPreviewLinkActiveOnTarget(component) {
        if(!component) return false;
        const linkedValues = computeAllLinkedValues();
        for(let s = 0; s < components.length; s++) {
            const srcComp = components[s];
            if(!srcComp || srcComp === component) continue;
            const links = Array.isArray(srcComp.linkedSlaves) ? srcComp.linkedSlaves : [];
            for(let li = 0; li < links.length; li++) {
                const link = links[li];
                if(!link || !link.enabled || !link.targetId || link.targetId !== component.id) continue;
                const srcValue = getPreviewEffectiveSourceValue(srcComp, linkedValues);
                if(computePreviewRangeActive('link', link, srcComp, component, srcValue)) return true;
            }
        }
        return false;
    }

    /**
     * 计算预览中组件的自动动画开启值（0/1/数值）。
     * 与导出生成的 Sync Bindings 一致：其他组件（开关/滑块/摇杆）可把 $auto_<索引>
     * 绑定为自己的输出变量，从而控制目标组件的自动动画。
     * 没有绑定者时返回 1（保持原行为：开启自动动画即播放）。
     */
    function getPreviewAutoEnableValue(component) {
        if(!component || component.autoAnimate !== true) return 0;
        const idx = components.indexOf(component);
        if(idx < 0) return 1;
        // 联动占用闩锁：联动激活期间自动动画暂停（与导出端 $linked_active_<idx> 一致，A8 修复）
        if(isPreviewLinkActiveOnTarget(component)) return 0;
        const autoVar = `$auto_${idx}`;
        let bound = null;
        components.forEach((src) => {
            if(!src || src === component) return;
            const vars = Array.isArray(src.vars) ? src.vars.flatMap(v => splitVarStr(v)) : [];
            if(!vars.includes(autoVar)) return;
            if(src.type === 'toggle') {
                const sig = Number(getPreviewComponentSignal(src, previewSimulationSnapshot)) || 0;
                const val = clamp(sig, 0, Math.max(1, Number(src.toggleSteps) || 1));
                if(src.switchGroup && src.switchGroup > 0) {
                    bound = val > 0 ? val : 0;
                } else if(src.toggleInvert === true) {
                    bound = val > 0 ? 0 : 1;
                } else {
                    bound = val > 0 ? val : 0;
                }
            } else if(src.type === 'slider_h' || src.type === 'slider_v') {
                const sig = Number(getSliderPreviewValue(src, previewSimulationSnapshot)) || 0;
                const maxVal = autoNumber((src.maxVals && src.maxVals[0]) ?? 1, 1);
                bound = clamp(sig, 0, 1) * maxVal;
            } else if(src.type === 'joystick') {
                const vec = getJoystickPreviewVector(src, previewSimulationSnapshot);
                const maxX = autoNumber((src.maxVals && src.maxVals[0]) ?? 1, 1);
                const maxY = autoNumber((src.maxVals && src.maxVals[1]) ?? 1, 1);
                const axis1 = (src.vars && src.vars[0]) ? splitVarStr(src.vars[0])[0] : null;
                const axis2 = (src.vars && src.vars[1]) ? splitVarStr(src.vars[1])[0] : null;
                if(axis1 === autoVar) bound = (vec.x || 0) * maxX;
                else if(axis2 === autoVar) bound = (vec.y || 0) * maxY;
                else bound = Math.max(Math.abs(vec.x || 0), Math.abs(vec.y || 0));
            }
        });
        return bound === null ? 1 : bound;
    }

    function stepPreviewSliderState(component, runtime) {
        const rest = getSliderPreviewBaseValue(component);
        const springK = autoNumber(component.springK, 0.05);
        const springD = autoNumber(component.springD, 0.95);
        const autoEnabled = getPreviewAutoEnableValue(component) === 1;
        const autoSource = getAutoSourceMode(component);
        const autoAmpX = autoNumber(component.autoAmpX, 1);
        const autoSpeed = Math.max(autoNumber(component.autoSpeed, 0.015), 0.08);
        const autoResponse = Math.max(autoNumber(component.autoResponse, 0.22), 0.35);
        const autoBounce = autoNumber(component.autoBounce, 0.25);
        const autoStr = autoNumber(component.autoStr, 0.1);
        const gravity = autoNumber(component.gravity, 0);
        const chaosRate = Math.max(1, Math.round(autoNumber(component.chaosRate, 96)));
        let force = 0;

        if(autoEnabled) {
            if(!runtime.autoPrev) {
                runtime.chaosX = autoNumber(component.autoSeedX, 0.3187);
                runtime.counter = Math.max(0, chaosRate - 1);
                runtime.autoPhase = 0;
                runtime.autoGoal = runtime.value;
                runtime.autoTgt = runtime.value;
                runtime.autoPrev = 1;
            }
            if(autoSource === 'chaos') {
                let blend = 8 / chaosRate;
                if(blend > 1) blend = 1;
                const chaosNext = 3.91 * runtime.chaosX * (1 - runtime.chaosX);
                runtime.chaosX = runtime.chaosX + (chaosNext - runtime.chaosX) * blend;
                if(component.paramMode === '2') {
                    runtime.autoGoal = 0.5 + (runtime.chaosX - 0.5) * autoAmpX;
                } else if(sliderUsesExplicitRange(component)) {
                    const mid = (getSliderRangeMin(component) + getSliderRangeMax(component)) * 0.5;
                    const halfSpan = getSliderRangeSpan(component) * 0.5;
                    const actualGoal = mid + (runtime.chaosX - 0.5) * 2 * halfSpan * autoAmpX;
                    runtime.autoGoal = sliderActualToNormalized(component, actualGoal);
                } else {
                    runtime.autoGoal = runtime.chaosX * autoAmpX;
                }
                runtime.autoGoal = clamp(runtime.autoGoal, 0, 1);
            } else {
                runtime.autoPhase = wrap01(runtime.autoPhase + (1 / chaosRate));
                runtime.autoGoal = clamp(readPreviewFunctionSample(runtime.samplesX, runtime.autoPhase), 0, 1);
            }
            let delta = (runtime.autoGoal - runtime.autoTgt) * autoResponse;
            delta = clamp(delta, -autoSpeed, autoSpeed);
            runtime.autoTgt += delta;
            const driveK = Math.max((springK * 2.2) + autoStr, 0.35);
            force = (runtime.autoTgt - runtime.value) * driveK;
        } else {
            runtime.autoPrev = 0;
            force = (rest - runtime.value) * springK;
        }

        if(gravity !== 0) force -= gravity;
        runtime.vel = (runtime.vel + force) * springD;
        runtime.value += runtime.vel;

        if(gravity !== 0) {
            if(runtime.value < 0) {
                runtime.value = 0;
                runtime.vel = -runtime.vel * autoBounce;
            }
            if(runtime.value > 1) {
                runtime.value = 1;
                runtime.vel = -runtime.vel * autoBounce;
            }
        } else if(autoEnabled) {
            if(runtime.value < 0) {
                runtime.value = 0;
                runtime.vel = -runtime.vel * autoBounce;
            }
            if(runtime.value > 1) {
                runtime.value = 1;
                runtime.vel = -runtime.vel * autoBounce;
            }
        } else {
            if(runtime.value < 0) {
                runtime.value = 0;
                runtime.vel = 0;
            }
            if(runtime.value > 1) {
                runtime.value = 1;
                runtime.vel = 0;
            }
        }
    }

    function stepPreviewJoystickState(component, runtime) {
        const rest = getJoystickPreviewBaseVector(component);
        const springK = autoNumber(component.springK, 0.05);
        const springD = autoNumber(component.springD, 0.95);
        const autoEnabled = getPreviewAutoEnableValue(component) === 1;
        const autoSource = getAutoSourceMode(component);
        const autoAmpX = autoNumber(component.autoAmpX, 1);
        const autoAmpY = autoNumber(component.autoAmpY, 1);
        const autoSpeed = Math.max(autoNumber(component.autoSpeed, 0.015), 0.08);
        const autoResponse = Math.max(autoNumber(component.autoResponse, 0.22), 0.35);
        const autoBounce = autoNumber(component.autoBounce, 0.25);
        const autoStr = autoNumber(component.autoStr, 0.1);
        const gravity = autoNumber(component.gravity, 0);
        const chaosRate = Math.max(1, Math.round(autoNumber(component.chaosRate, 96)));

        if(component.physicsProfile === 'breast') {
            let driveX = rest.x;
            let driveY = rest.y;
            if(autoEnabled) {
                if(!runtime.autoPrev) {
                    runtime.chaosX = autoNumber(component.autoSeedX, 0.3187);
                    runtime.chaosY = autoNumber(component.autoSeedY, 0.6123);
                    runtime.counter = Math.max(0, chaosRate - 1);
                    runtime.autoPhase = 0;
                    runtime.autoGoalX = runtime.valueX;
                    runtime.autoGoalY = runtime.valueY;
                    runtime.autoTgtX = runtime.valueX;
                    runtime.autoTgtY = runtime.valueY;
                    runtime.autoPrev = 1;
                }
                if(autoSource === 'chaos') {
                    runtime.counter += 1;
                    if(runtime.counter >= chaosRate) {
                        runtime.counter = 0;
                        runtime.chaosX = 3.91 * runtime.chaosX * (1 - runtime.chaosX);
                        runtime.chaosY = 3.83 * runtime.chaosY * (1 - runtime.chaosY);
                        runtime.autoGoalX = clamp((runtime.chaosX - 0.5) * 2 * autoAmpX, -1, 1);
                        runtime.autoGoalY = clamp((runtime.chaosY - 0.5) * 2 * autoAmpY, -1, 1);
                    }
                } else {
                    runtime.autoPhase = wrap01(runtime.autoPhase + (1 / chaosRate));
                    runtime.autoGoalX = clamp(readPreviewFunctionSample(runtime.samplesX, runtime.autoPhase), -1, 1);
                    runtime.autoGoalY = clamp(readPreviewFunctionSample(runtime.samplesY, runtime.autoPhase), -1, 1);
                }
                let deltaX = (runtime.autoGoalX - runtime.autoTgtX) * autoResponse;
                let deltaY = (runtime.autoGoalY - runtime.autoTgtY) * autoResponse;
                deltaX = clamp(deltaX, -autoSpeed, autoSpeed);
                deltaY = clamp(deltaY, -autoSpeed, autoSpeed);
                runtime.autoTgtX += deltaX;
                runtime.autoTgtY += deltaY;
                driveX = runtime.autoTgtX;
                driveY = runtime.autoTgtY;
            } else {
                runtime.autoPrev = 0;
            }

            let forceX = (driveX - runtime.valueX) * ((springK * 1.55) + 0.12);
            runtime.velX = (runtime.velX + forceX) * ((springD * 0.72) + 0.16);
            runtime.valueX += runtime.velX;

            let forceY = (driveY - runtime.valueY) * ((springK * 2.05) + 0.16);
            let temp = driveY - runtime.valueY;
            if(temp < 0) temp = -temp;
            forceY += temp * ((springK * 0.30) + 0.03);
            temp = runtime.velY;
            if(temp < 0) temp = -temp;
            forceY += temp * ((springK * 0.08) + 0.01);
            if(gravity !== 0) forceY -= gravity;
            runtime.velY = (runtime.velY + forceY) * ((springD * 0.66) + 0.18);
            runtime.valueY += runtime.velY;

            if(runtime.valueX < -1) {
                runtime.valueX = -1;
                runtime.velX = -runtime.velX * autoBounce;
            }
            if(runtime.valueX > 1) {
                runtime.valueX = 1;
                runtime.velX = -runtime.velX * autoBounce;
            }
            if(runtime.valueY < -1) {
                runtime.valueY = -1;
                runtime.velY = -runtime.velY * autoBounce;
            }
            if(runtime.valueY > 1) {
                runtime.valueY = 1;
                runtime.velY = -runtime.velY * autoBounce;
            }
            return;
        }

        if(autoEnabled) {
            if(!runtime.autoPrev) {
                runtime.chaosX = autoNumber(component.autoSeedX, 0.3187);
                runtime.chaosY = autoNumber(component.autoSeedY, 0.6123);
                runtime.counter = Math.max(0, chaosRate - 1);
                runtime.autoPhase = 0;
                runtime.autoGoalX = runtime.valueX;
                runtime.autoGoalY = runtime.valueY;
                runtime.autoTgtX = runtime.valueX;
                runtime.autoTgtY = runtime.valueY;
                runtime.autoPrev = 1;
            }
            if(autoSource === 'chaos') {
                let blend = 8 / chaosRate;
                if(blend > 1) blend = 1;
                let nextX = 3.91 * runtime.chaosX * (1 - runtime.chaosX);
                runtime.chaosX = runtime.chaosX + (nextX - runtime.chaosX) * blend;
                let nextY = 3.83 * runtime.chaosY * (1 - runtime.chaosY);
                runtime.chaosY = runtime.chaosY + (nextY - runtime.chaosY) * blend;
                runtime.autoGoalX = clamp((runtime.chaosX - 0.5) * 2 * autoAmpX, -1, 1);
                runtime.autoGoalY = clamp((runtime.chaosY - 0.5) * 2 * autoAmpY, -1, 1);
            } else {
                runtime.autoPhase = wrap01(runtime.autoPhase + (1 / chaosRate));
                runtime.autoGoalX = clamp(readPreviewFunctionSample(runtime.samplesX, runtime.autoPhase), -1, 1);
                runtime.autoGoalY = clamp(readPreviewFunctionSample(runtime.samplesY, runtime.autoPhase), -1, 1);
            }
            let deltaX = (runtime.autoGoalX - runtime.autoTgtX) * autoResponse;
            let deltaY = (runtime.autoGoalY - runtime.autoTgtY) * autoResponse;
            deltaX = clamp(deltaX, -autoSpeed, autoSpeed);
            deltaY = clamp(deltaY, -autoSpeed, autoSpeed);
            runtime.autoTgtX += deltaX;
            runtime.autoTgtY += deltaY;
            const driveK = Math.max((springK * 2.2) + autoStr, 0.35);
            let forceX = (runtime.autoTgtX - runtime.valueX) * driveK;
            runtime.velX = (runtime.velX + forceX) * springD;
            runtime.valueX += runtime.velX;
            let forceY = (runtime.autoTgtY - runtime.valueY) * driveK;
            if(gravity !== 0) forceY -= gravity;
            runtime.velY = (runtime.velY + forceY) * springD;
            runtime.valueY += runtime.velY;
        } else {
            runtime.autoPrev = 0;
            let forceX = (rest.x - runtime.valueX) * springK;
            runtime.velX = (runtime.velX + forceX) * springD;
            runtime.valueX += runtime.velX;
            let forceY = (rest.y - runtime.valueY) * springK;
            if(gravity !== 0) forceY -= gravity;
            runtime.velY = (runtime.velY + forceY) * springD;
            runtime.valueY += runtime.velY;
        }

        if(gravity !== 0) {
            if(runtime.valueX < -1) {
                runtime.valueX = -1;
                runtime.velX = -runtime.velX * autoBounce;
            }
            if(runtime.valueX > 1) {
                runtime.valueX = 1;
                runtime.velX = -runtime.velX * autoBounce;
            }
            if(runtime.valueY < -1) {
                runtime.valueY = -1;
                runtime.velY = -runtime.velY * autoBounce;
            }
            if(runtime.valueY > 1) {
                runtime.valueY = 1;
                runtime.velY = -runtime.velY * autoBounce;
            }
        } else if(autoEnabled) {
            if(runtime.valueX < -1) {
                runtime.valueX = -1;
                runtime.velX = -runtime.velX * autoBounce;
            }
            if(runtime.valueX > 1) {
                runtime.valueX = 1;
                runtime.velX = -runtime.velX * autoBounce;
            }
            if(runtime.valueY < -1) {
                runtime.valueY = -1;
                runtime.velY = -runtime.velY * autoBounce;
            }
            if(runtime.valueY > 1) {
                runtime.valueY = 1;
                runtime.velY = -runtime.velY * autoBounce;
            }
        } else {
            if(runtime.valueX < -1) {
                runtime.valueX = -1;
                runtime.velX = 0;
            }
            if(runtime.valueX > 1) {
                runtime.valueX = 1;
                runtime.velX = 0;
            }
            if(runtime.valueY < -1) {
                runtime.valueY = -1;
                runtime.velY = 0;
            }
            if(runtime.valueY > 1) {
                runtime.valueY = 1;
                runtime.velY = 0;
            }
        }
    }

    function getPreviewComponentSignal(component, snapshot = previewSimulationSnapshot) {
        if(!component) return null;
        const state = snapshot && snapshot.componentStates ? snapshot.componentStates.get(component.id) : null;
        if(component.type === 'joystick') {
            const base = getJoystickPreviewBaseVector(component);
            return {
                x: clamp(state && Number.isFinite(state.valueX) ? state.valueX : base.x, -1, 1),
                y: clamp(state && Number.isFinite(state.valueY) ? state.valueY : base.y, -1, 1)
            };
        }
        if(component.type === 'slider_h' || component.type === 'slider_v') {
            const base = getSliderPreviewBaseValue(component);
            return clamp(state && Number.isFinite(state.value) ? state.value : base, 0, 1);
        }
        if(component.type === 'toggle') {
            return clamp(Number(component.initialValue) || 0, 0, isToggleMultiMode(component) ? (component.toggleSteps || DEFAULT_TOGGLE_STEPS) : 1);
        }
        return null;
    }

    function getPreviewJoystickDirectionValues(component, vector) {
        if(!component || component.type !== 'joystick' || component.paramMode !== '4') return [];
        const joyCfg = getJoystickConfig(component);
        const values = new Array(joyCfg.directionCount * joyCfg.subdivisions).fill(0);
        const weights = getJoystickDirectionalBlendWeights(component, vector);
        for(let dirIdx = 0; dirIdx < joyCfg.directionCount; dirIdx++) {
            writeJoystickDirectionPressureValue(component, values, dirIdx, weights[dirIdx] || 0);
        }
        return values;
    }

    function getSliderPreviewGridValue(component, value) {
        const gridSteps = clampGridStepCount(component.gridSteps, 3);
        if(gridSteps === 1) return getSliderGridOutputValue(component, 0);
        const interval = 1 / (gridSteps - 1);
        for(let s = 0; s < gridSteps; s++) {
            if(s === gridSteps - 1) return getSliderGridOutputValue(component, s);
            const threshold = (s * interval) + interval * 0.5;
            if(value < threshold) return getSliderGridOutputValue(component, s);
        }
        return getSliderGridOutputValue(component, gridSteps - 1);
    }

    function buildPreviewVariableValues(snapshot) {
        const vars = new Map();
        previewActionVarValues.forEach((value, key) => {
            vars.set(key, value);
        });
        // Phase 1: 计算各组件自身变量值
        components.forEach((component) => {
            if(!component) return;
            if(component.type === 'toggle') {
                const current = getPreviewComponentSignal(component, snapshot);
                const targets = (component.vars || []).flatMap(v => splitVarStr(v));
                if(component.switchGroup && component.switchGroup > 0) {
                    if(current > 0) targets.forEach((v) => vars.set(v, current));
                } else {
                    targets.forEach((v) => vars.set(v, current));
                }
                return;
            }
            if(component.type === 'slider_h' || component.type === 'slider_v') {
                const current = getPreviewComponentSignal(component, snapshot);
                if(isSliderSubdivisionMode(component)) {
                    const segmentValues = getSliderSubdivisionValues(component, current);
                    segmentValues.forEach((segmentValue, flatIdx) => {
                        const targetVarStr = component.vars && component.vars[flatIdx] ? component.vars[flatIdx].trim() : '';
                        splitVarStr(targetVarStr).forEach(v => vars.set(v, segmentValue));
                    });
                } else if(component.paramMode === '2') {
                    const leftVarStr = component.vars && component.vars[0] ? component.vars[0].trim() : '';
                    const rightVarStr = component.vars && component.vars[1] ? component.vars[1].trim() : '';
                    const leftVars = splitVarStr(leftVarStr);
                    const rightVars = splitVarStr(rightVarStr);
                    const maxVal = autoNumber((component.maxVals && component.maxVals[0]) ?? 1, 1);
                    leftVars.forEach(v => vars.set(v, Math.max(0, (0.5 - current) * 2 * maxVal)));
                    rightVars.forEach(v => vars.set(v, Math.max(0, (current - 0.5) * 2 * maxVal)));
                } else if(isSliderGridMode(component)) {
                    const gridValue = getSliderPreviewGridValue(component, current);
                    getSliderGridMainVars(component).forEach((mainVar) => vars.set(mainVar, gridValue));
                } else {
                    const mainVarStr = component.vars && component.vars[0] ? component.vars[0].trim() : '';
                    const mainVars = splitVarStr(mainVarStr);
                    const maxVal = autoNumber((component.maxVals && component.maxVals[0]) ?? 1, 1);
                    const outputValue = sliderUsesExplicitRange(component) ? sliderNormalizedToActual(component, current) : (current * maxVal);
                    mainVars.forEach(v => vars.set(v, outputValue));
                }
                return;
            }
            if(component.type === 'joystick') {
                const current = getPreviewComponentSignal(component, snapshot);
                if(component.paramMode === '4') {
                    const joyDirValues = getPreviewJoystickDirectionValues(component, current);
                    joyDirValues.forEach((dirVal, flatIdx) => {
                        const targetVarStr = component.vars && component.vars[flatIdx] ? component.vars[flatIdx].trim() : '';
                        const targetVars = splitVarStr(targetVarStr);
                        const maxVal = autoNumber((component.maxVals && component.maxVals[flatIdx]) ?? 1, 1);
                        targetVars.forEach(v => vars.set(v, dirVal * maxVal));
                    });
                } else {
                    const varXStr = component.vars && component.vars[0] ? component.vars[0].trim() : '';
                    const varYStr = component.vars && component.vars[1] ? component.vars[1].trim() : '';
                    const varXVars = splitVarStr(varXStr);
                    const varYVars = splitVarStr(varYStr);
                    const maxX = autoNumber((component.maxVals && component.maxVals[0]) ?? 1, 1);
                    const maxY = autoNumber((component.maxVals && component.maxVals[1]) ?? 1, 1);
                    varXVars.forEach(v => vars.set(v, current.x * maxX));
                    varYVars.forEach(v => vars.set(v, current.y * maxY));
                }
            }
        });
        // Phase 2: 应用嵌套联动覆盖 → 将 computeAllLinkedValues 的计算结果写入变量表
        let linkedValues;
        try {
            linkedValues = computeAllLinkedValues();
        } catch(e) {
            console.error('computeAllLinkedValues in buildPreviewVariableValues error:', e);
            linkedValues = new Map();
        }
        updatePreviewNestedActionStates(linkedValues);
        previewActionVarValues.forEach((value, key) => {
            vars.set(key, value);
        });
        applyPreviewDependencyTriggerValues(vars, snapshot);
        linkedValues.forEach((mappedValue, targetId) => {
            const targetComp = components.find(c => c.id === targetId);
            if(!targetComp) return;
            if(targetComp.type === 'joystick') {
                if(targetComp.paramMode === '4') {
                    // 方向摇杆: 将覆写值写入各方向对应的变量
                    if(mappedValue && typeof mappedValue === 'object' && typeof mappedValue.x === 'number') {
                        // 路径1: 从合成后的 {x,y} 重建各方向压感值 (eg, XY源→方向摇杆)
                        const tgtDirValues = getPreviewJoystickDirectionValues(targetComp, mappedValue);
                        const cfg = getJoystickConfig(targetComp);
                        for(let dirIdx = 0; dirIdx < cfg.directionCount; dirIdx++) {
                            const pressureTarget = {};
                            writeJoystickDirectionPressureValue(targetComp, pressureTarget, dirIdx, getJoystickDirectionPressureValue(targetComp, tgtDirValues, dirIdx));
                            for(let segIdx = 0; segIdx < cfg.subdivisions; segIdx++) {
                                const flatIdx = getJoystickDirectionVarIndex(targetComp, dirIdx, segIdx);
                                const targetVarStr = targetComp.vars && targetComp.vars[flatIdx] ? targetComp.vars[flatIdx].trim() : '';
                                const targetVars = splitVarStr(targetVarStr);
                                const maxVal = autoNumber((targetComp.maxVals && targetComp.maxVals[flatIdx]) ?? 1, 1);
                                let dirVal = pressureTarget[flatIdx] ?? 0;
                                // 路径2: 单方向直接覆写 → 覆盖重建值 (eg, 滑块→方向摇杆个别方向)
                                if(typeof mappedValue[flatIdx] === 'number') {
                                    dirVal = mappedValue[flatIdx];
                                }
                                targetVars.forEach(v => vars.set(v, dirVal * maxVal));
                            }
                        }
                    } else if(mappedValue && typeof mappedValue === 'object') {
                        // 路径3: 仅有逐方向覆写值, 无 {x,y} (eg, 普通滑条→方向摇杆)
                        const cfg = getJoystickConfig(targetComp);
                        for(let flatIdx = 0; flatIdx < cfg.directionCount * cfg.subdivisions; flatIdx++) {
                            if(typeof mappedValue[flatIdx] === 'number') {
                                const targetVarStr = targetComp.vars && targetComp.vars[flatIdx] ? targetComp.vars[flatIdx].trim() : '';
                                const targetVars = splitVarStr(targetVarStr);
                                const maxVal = autoNumber((targetComp.maxVals && targetComp.maxVals[flatIdx]) ?? 1, 1);
                                targetVars.forEach(v => vars.set(v, mappedValue[flatIdx] * maxVal));
                            }
                        }
                    }
                } else {
                    // XY摇杆: 将覆写值 {x,y} 分别写入 X/Y 变量
                    const varXStr = targetComp.vars && targetComp.vars[0] ? targetComp.vars[0].trim() : '';
                    const varYStr = targetComp.vars && targetComp.vars[1] ? targetComp.vars[1].trim() : '';
                    const varXVars = splitVarStr(varXStr);
                    const varYVars = splitVarStr(varYStr);
                    const maxX = autoNumber((targetComp.maxVals && targetComp.maxVals[0]) ?? 1, 1);
                    const maxY = autoNumber((targetComp.maxVals && targetComp.maxVals[1]) ?? 1, 1);
                    const val = typeof mappedValue === 'object' ? mappedValue : { x: mappedValue, y: 0 };
                    varXVars.forEach(v => { if(typeof val.x === 'number') vars.set(v, val.x * maxX); });
                    varYVars.forEach(v => { if(typeof val.y === 'number') vars.set(v, val.y * maxY); });
                }
            } else if(targetComp.type === 'slider_h' || targetComp.type === 'slider_v') {
                // 滑块: 将覆写值写入对应的变量
                const linkedVal = typeof mappedValue === 'number' ? mappedValue : 
                    (typeof mappedValue === 'object' ? (mappedValue.x ?? 0) : 0);
                if(isSliderSubdivisionMode(targetComp)) {
                    const segmentValues = getSliderSubdivisionValues(targetComp, linkedVal);
                    segmentValues.forEach((segmentValue, flatIdx) => {
                        const targetVarStr = targetComp.vars && targetComp.vars[flatIdx] ? targetComp.vars[flatIdx].trim() : '';
                        splitVarStr(targetVarStr).forEach(v => vars.set(v, segmentValue));
                    });
                } else if(targetComp.paramMode === '2') {
                    // 双向滑条: 拆分为左/右两个变量 (左值+右值=1)
                    const leftVarStr = targetComp.vars && targetComp.vars[0] ? targetComp.vars[0].trim() : '';
                    const rightVarStr = targetComp.vars && targetComp.vars[1] ? targetComp.vars[1].trim() : '';
                    const leftVars = splitVarStr(leftVarStr);
                    const rightVars = splitVarStr(rightVarStr);
                    const maxVal = autoNumber((targetComp.maxVals && targetComp.maxVals[0]) ?? 1, 1);
                    leftVars.forEach(v => vars.set(v, Math.max(0, (0.5 - linkedVal) * 2 * maxVal)));
                    rightVars.forEach(v => vars.set(v, Math.max(0, (linkedVal - 0.5) * 2 * maxVal)));
                } else if(isSliderGridMode(targetComp)) {
                    const gridVal = getSliderPreviewGridValue(targetComp, linkedVal);
                    getSliderGridMainVars(targetComp).forEach((mainVar) => vars.set(mainVar, gridVal));
                } else {
                    const mainVarStr = targetComp.vars && targetComp.vars[0] ? targetComp.vars[0].trim() : '';
                    const mainVars = splitVarStr(mainVarStr);
                    const maxVal = autoNumber((targetComp.maxVals && targetComp.maxVals[0]) ?? 1, 1);
                    const outputValue = sliderUsesExplicitRange(targetComp) ? sliderNormalizedToActual(targetComp, linkedVal) : (linkedVal * maxVal);
                    mainVars.forEach(v => vars.set(v, outputValue));
                }
            }
        });
        return vars;
    }

    function rebuildPreviewSimulationSnapshot(nowSec = getPreviewTimeSeconds()) {
        const componentStates = new Map();
        const activeIds = new Set();
        components.forEach((component) => {
            if(!component || !component.id) return;
            activeIds.add(component.id);
            const state = getPreviewInteractiveState(component, nowSec);
            if(state) componentStates.set(component.id, state);
        });
        Array.from(previewInteractiveRuntime.keys()).forEach((id) => {
            if(!activeIds.has(id)) previewInteractiveRuntime.delete(id);
        });
        previewSimulationSnapshot = {
            time: nowSec,
            componentStates,
            variableValues: null
        };
        previewSimulationSnapshot.variableValues = buildPreviewVariableValues(previewSimulationSnapshot);
        return previewSimulationSnapshot;
    }

    function getPreviewSimulationSnapshot(nowSec = getPreviewTimeSeconds()) {
        if(!previewSimulationSnapshot || Math.abs((previewSimulationSnapshot.time || 0) - nowSec) > 0.0005) {
            return rebuildPreviewSimulationSnapshot(nowSec);
        }
        return previewSimulationSnapshot;
    }

    function getSliderPreviewValue(component, snapshot = previewSimulationSnapshot) {
        const signal = getPreviewComponentSignal(component, snapshot);
        return typeof signal === 'number' ? signal : getSliderPreviewBaseValue(component);
    }

    function getJoystickPreviewVector(component, snapshot = previewSimulationSnapshot) {
        const signal = getPreviewComponentSignal(component, snapshot);
        if(signal && typeof signal === 'object') return signal;
        return getJoystickPreviewBaseVector(component);
    }

    function getSequencePreviewFrame(component, snapshot = previewSimulationSnapshot) {
        const frames = Array.isArray(component && component.frames) ? component.frames : [];
        if(frames.length === 0) return null;
        const seqVar = component && component.seqVar ? component.seqVar.trim() : '';
        const variableValues = snapshot && snapshot.variableValues ? snapshot.variableValues : new Map();
        const resolved = seqVar && variableValues.has(seqVar) ? variableValues.get(seqVar) : null;
        if(resolved != null) {
            const numericResolved = Number(resolved);
            const matched = frames.find((frame) => String(frame.val) === String(resolved) || (Number.isFinite(numericResolved) && Number.isFinite(Number(frame.val)) && Math.abs(Number(frame.val) - numericResolved) < 0.000001));
            if(matched) return matched;
        }
        return frames.find((frame) => Number(frame.val) === 0 && frame.preview) || frames.find((frame) => frame.preview) || frames[0];
    }

    function formatPreviewVariableValue(value) {
        const num = Number(value);
        if(Number.isFinite(num)) {
            if(Math.abs(num - Math.round(num)) < 0.000001) return String(Math.round(num));
            return num.toFixed(3).replace(/0+$/, '').replace(/\.$/, '');
        }
        return value == null ? '0' : String(value);
    }

    function getReadableTextColor(backgroundColor) {
        const value = String(backgroundColor || '').trim();
        let rgb = null;
        const hexMatch = value.match(/^#([0-9a-f]{3}|[0-9a-f]{6})$/i);
        if(hexMatch) {
            const hex = hexMatch[1].length === 3
                ? hexMatch[1].split('').map(char => char + char).join('')
                : hexMatch[1];
            rgb = [0, 2, 4].map(offset => parseInt(hex.slice(offset, offset + 2), 16));
        } else {
            const rgbMatch = value.match(/^rgba?\(\s*(\d+)\s*,\s*(\d+)\s*,\s*(\d+)/i);
            if(rgbMatch) rgb = rgbMatch.slice(1, 4).map(Number);
        }
        if(!rgb || rgb.some(channel => !Number.isFinite(channel))) return '#ffffff';
        const linear = channel => {
            const normalized = Math.max(0, Math.min(255, channel)) / 255;
            return normalized <= 0.04045
                ? normalized / 12.92
                : Math.pow((normalized + 0.055) / 1.055, 2.4);
        };
        const luminance = 0.2126 * linear(rgb[0]) + 0.7152 * linear(rgb[1]) + 0.0722 * linear(rgb[2]);
        const blackContrast = (luminance + 0.05) / 0.05;
        const whiteContrast = 1.05 / (luminance + 0.05);
        return blackContrast >= whiteContrast ? '#000000' : '#ffffff';
    }

    function formatTextRuntimeValue(value) {
        const number = Number(value);
        const integer = clamp(Number.isFinite(number) ? Math.floor(number) : 0, 0, 999);
        return String(integer).padStart(3, '0');
    }

    function getTextPreviewReplacement(component, snapshot = previewSimulationSnapshot) {
        if(!component || !component.valVar) return '100';
        const variableValues = snapshot && snapshot.variableValues ? snapshot.variableValues : new Map();
        if(variableValues.has(component.valVar)) return formatTextRuntimeValue(variableValues.get(component.valVar));
        return '000';
    }

    function clampJoystickVectorToRoundedBounds(component, x, y) {
        const rect = getComponentPixelRect(component || { w: 0.1, h: 0.1 });
        const handle = getComponentHandleMetrics(component || { w: 0.1, h: 0.1 });
        const halfRangeX = Math.max(0.0001, Math.max(0, rect.width - handle.width) * 0.5);
        const halfRangeY = Math.max(0.0001, Math.max(0, rect.height - handle.height) * 0.5);
        let nx = clamp(Number(x) || 0, -1, 1);
        let ny = clamp(Number(y) || 0, -1, 1);
        let px = clamp(nx * halfRangeX, -halfRangeX, halfRangeX);
        let py = clamp(-ny * halfRangeY, -halfRangeY, halfRangeY);
        const outerRadius = Math.max(0, Math.min(getComponentCornerRadiusPx(component || {}), Math.min(rect.width, rect.height) * 0.5));
        const handleRadius = Math.max(0, Math.min(handle.width, handle.height) * 0.5);
        const centerRadius = Math.max(0, Math.min(Math.min(halfRangeX, halfRangeY), outerRadius - handleRadius));
        const innerHalfX = Math.max(0, halfRangeX - centerRadius);
        const innerHalfY = Math.max(0, halfRangeY - centerRadius);
        if(centerRadius > 0 && Math.abs(px) > innerHalfX && Math.abs(py) > innerHalfY) {
            const cornerX = Math.sign(px || 1) * innerHalfX;
            const cornerY = Math.sign(py || 1) * innerHalfY;
            const offsetX = px - cornerX;
            const offsetY = py - cornerY;
            const offsetLen = Math.sqrt(offsetX * offsetX + offsetY * offsetY);
            if(offsetLen > centerRadius && offsetLen > 0.000001) {
                const scale = centerRadius / offsetLen;
                px = cornerX + offsetX * scale;
                py = cornerY + offsetY * scale;
            }
        }
        return {
            x: clamp(px / halfRangeX, -1, 1),
            y: clamp((-py) / halfRangeY, -1, 1)
        };
    }

    let textMeasureContext = null;
    const textGlyphRatioCache = new Map();

    function getTextFontCss(component, fontSizePx = 100) {
        const family = String(component && component.fontFamily || 'Microsoft YaHei').replace(/["\\]/g, '');
        return `${component && component.fontItalic ? 'italic ' : ''}${component && component.fontBold ? 'bold ' : ''}${fontSizePx}px "${family}"`;
    }

    function getTextMeasureContext() {
        if(textMeasureContext) return textMeasureContext;
        const canvas = document.createElement('canvas');
        textMeasureContext = canvas.getContext('2d');
        return textMeasureContext;
    }

    function getTextGlyphAdvanceRatio(component, char) {
        if(char === '\n') return 0;
        const cacheKey = `${getTextFontSignature(component)}:${char}`;
        if(textGlyphRatioCache.has(cacheKey)) return textGlyphRatioCache.get(cacheKey);
        const ctx = getTextMeasureContext();
        if(!ctx) {
            const fallbackRatio = /^[\x00-\xff]$/.test(char) ? 0.58 : 1;
            textGlyphRatioCache.set(cacheKey, fallbackRatio);
            return fallbackRatio;
        }
        const measureSize = 100;
        ctx.font = getTextFontCss(component, measureSize);
        const measured = ctx.measureText(char || ' ').width / measureSize;
        const fallback = /^[\x00-\xff]$/.test(char) ? 0.58 : 1;
        const ratio = clamp(Number.isFinite(measured) && measured > 0 ? measured : fallback, 0.24, 1.25);
        textGlyphRatioCache.set(cacheKey, ratio);
        return ratio;
    }

    function getTextDigitMaxAdvanceRatio(component) {
        let maxRatio = 0;
        for(let digit = 0; digit <= 9; digit++) {
            maxRatio = Math.max(maxRatio, getTextGlyphAdvanceRatio(component, String(digit)));
        }
        return Math.max(0.5, maxRatio);
    }

    function getTextFontSignature(component) {
        return (hashStableString({
            family: component && component.fontFamily || 'Microsoft YaHei',
            bold: !!(component && component.fontBold),
            italic: !!(component && component.fontItalic)
        }) >>> 0).toString(16).toUpperCase();
    }

    function getTextGlyphResourceKey(component, char, color) {
        const hex = char.codePointAt(0).toString(16).toUpperCase();
        const colorHex = String(color || '#ffffff').replace('#', '').toUpperCase();
        return `${hex}_${colorHex}_${getTextFontSignature(component)}`;
    }

    function buildRenderedTextLayout(component, options = {}) {
        const replacement = options.valueReplacement == null ? '100' : String(options.valueReplacement);
        const tokens = getComponentRenderedTextTokens(component, replacement);
        const text = tokens.map(token => token.char).join('');
        const isVertical = component && component.textFlow === 'vertical';
        const area = getWorkAreaPixelSize();
        const rect = getComponentPixelRect(component || { w: 0, h: 0 });
        const charStepX = safeNum(component && component.charSize, 0.03);
        const charStepY = charStepX * (area.width / Math.max(area.height, 1));
        const lineGapUnits = Math.max(0, safeNum(component && component.lineGap, 0.0));
        const lineAdvanceX = charStepX + lineGapUnits;
        const lineAdvanceY = charStepY + (lineGapUnits * (area.width / Math.max(area.height, 1)));
        const charPx = Math.max(4, Math.round(charStepX * area.width));
        const charHeightPx = Math.max(4, Math.round(charStepY * area.height));
        const gapPxX = Math.max(0, Math.round(lineGapUnits * area.width));
        const gapPxY = Math.max(0, Math.round((lineGapUnits * (area.width / Math.max(area.height, 1))) * area.height));
        const maxCols = Math.max(1, Math.floor((component && component.w ? component.w : 0) / Math.max(charStepX, 0.0001)));
        const maxRows = Math.max(1, Math.floor((component && component.h ? component.h : 0) / Math.max(charStepY, 0.0001)));
        const layout = [];
        const overrides = (component && component.colorOverrides) || {};
        const defaultColor = (component && component.fontColor) || '#ffffff';
        const dynamicRatio = getTextDigitMaxAdvanceRatio(component);
        let cursorX = 0;
        let cursorY = 0;

        const pushLineBreak = () => {
            if(isVertical) {
                cursorY = 0;
                cursorX += lineAdvanceX;
            } else {
                cursorX = 0;
                cursorY += lineAdvanceY;
            }
        };

        tokens.forEach((token) => {
            const char = token.char;
            if(char === '\n') {
                pushLineBreak();
                return;
            }
            const glyphRatio = token.isDynamic ? dynamicRatio : getTextGlyphAdvanceRatio(component, char);
            const glyphWidthUnit = charStepX * glyphRatio;
            const shouldWrap = isVertical
                ? cursorY + charStepY > (component && component.h ? component.h : 0) + 1e-6
                : cursorX + glyphWidthUnit > (component && component.w ? component.w : 0) + 1e-6;
            if(shouldWrap) pushLineBreak();
            if(isVertical && cursorX + charStepX > (component && component.w ? component.w : 0) + 1e-6) return;
            if(!isVertical && cursorY + charStepY > (component && component.h ? component.h : 0) + 1e-6) return;

            const xUnit = isVertical ? cursorX + (charStepX - glyphWidthUnit) * 0.5 : cursorX;
            const widthPx = Math.max(2, Math.round(glyphWidthUnit * area.width));
            layout.push({
                char,
                color: token.isDynamic ? defaultColor : (overrides[token.sourceIndex] || defaultColor),
                xUnit,
                yUnit: cursorY,
                leftPx: Math.round(xUnit * area.width),
                topPx: Math.round(cursorY * area.height),
                widthPx,
                heightPx: charHeightPx,
                fontSizePx: Math.max(10, Math.round(charHeightPx * 0.84)),
                glyphRatio,
                isDynamic: token.isDynamic,
                dynamicDigitIndex: token.dynamicDigitIndex,
                dynamicDigitCount: token.dynamicDigitCount,
                index: token.sourceIndex
            });
            if(isVertical) cursorY += charStepY;
            else cursorX += glyphWidthUnit;
        });

        return { text, tokens, isVertical, charPx, charHeightPx, gapPxX, gapPxY, maxCols, maxRows, rect, layout };
    }

    let uniqueTokenCounter = 0;
    function nextUniqueToken() {
        uniqueTokenCounter = (uniqueTokenCounter + 1) % 1000000;
        return `${Date.now()}_${uniqueTokenCounter}`;
    }

    function fract(v) {
        return v - Math.floor(v);
    }

    function mix(a, b, t) {
        return a + (b - a) * t;
    }

    function step(edge, x) {
        return x < edge ? 0 : 1;
    }

    function smoothstep(edge0, edge1, x) {
        if(edge0 === edge1) return x < edge0 ? 0 : 1;
        let t = clamp((x - edge0) / (edge1 - edge0), 0, 1);
        return t * t * (3 - 2 * t);
    }

    function saw(t) {
        return fract(t);
    }

    function tri(t) {
        return 1 - Math.abs(fract(t) * 2 - 1);
    }

    function pingpong(t) {
        return tri(t);
    }

    function sin01(t) {
        return 0.5 + 0.5 * Math.sin(Math.PI * 2 * t);
    }

    function cos01(t) {
        return 0.5 + 0.5 * Math.cos(Math.PI * 2 * t);
    }

    function compileAutoExpressionLegacy(expr) {
        const source = (expr || '').trim();
        if(!source) return null;
        if(/[^0-9A-Za-z_+\-*/%().,\s]/.test(source)) {
            throw new Error(`表达式包含不支持的字符: ${source}`);
        }
        const identifiers = source.match(/[A-Za-z_][A-Za-z0-9_]*/g) || [];
        for(const id of identifiers) {
            if(!AUTO_FUNCTION_ALLOWED_IDENTIFIERS.has(id)) {
                throw new Error(`表达式包含未授权标识符: ${id}`);
            }
        }
        const fn = new Function('ctx', `"use strict"; const { t, PI, TAU, E, sin, cos, tan, abs, min, max, pow, sqrt, log, exp, floor, ceil, round, sign, clamp, fract, mix, step, smoothstep, saw, tri, pingpong, sin01, cos01 } = ctx; return (${source});`);
        return (t) => {
            const ctx = {
                t,
                PI: Math.PI,
                TAU: Math.PI * 2,
                E: Math.E,
                sin: Math.sin,
                cos: Math.cos,
                tan: Math.tan,
                abs: Math.abs,
                min: Math.min,
                max: Math.max,
                pow: Math.pow,
                sqrt: Math.sqrt,
                log: Math.log,
                exp: Math.exp,
                floor: Math.floor,
                ceil: Math.ceil,
                round: Math.round,
                sign: Math.sign,
                clamp,
                fract,
                mix,
                step,
                smoothstep,
                saw,
                tri,
                pingpong,
                sin01,
                cos01
            };
            let value = Number(fn(ctx));
            if(!Number.isFinite(value)) value = 0;
            return value;
        };
    }

    function buildAutoFunctionSamples(component, axis = 'x') {
        const expr = axis === 'y' ? (component.autoFuncY || 'cos(TAU * t)') : (component.autoFuncX || 'sin01(t)');
        const evaluator = compileAutoExpression(expr);
        if(!evaluator) return new Array(AUTO_FUNCTION_SAMPLE_COUNT).fill(0);
        const samples = [];
        for(let idx = 0; idx < AUTO_FUNCTION_SAMPLE_COUNT; idx++) {
            const t = idx / AUTO_FUNCTION_SAMPLE_COUNT;
            let value = evaluator(t);
            if(component.type === 'joystick') {
                const amp = axis === 'y' ? (component.autoAmpY ?? 1) : (component.autoAmpX ?? 1);
                value = clamp(value * amp, -1, 1);
            } else if(component.type.includes('slider') && component.paramMode === '2') {
                value = clamp(0.5 + (value - 0.5) * (component.autoAmpX ?? 1), 0, 1);
            } else {
                value = clamp(value * (component.autoAmpX ?? 1), 0, 1);
            }
            samples.push(value);
        }
        return samples;
    }

    function buildAutoSampleAssignment(goalVar, phaseVar, samples) {
        if(!Array.isArray(samples) || samples.length === 0) return '';
        let out = '';
        const count = samples.length;
        samples.forEach((sample, idx) => {
            const line = `${goalVar} = ${sample.toFixed(6)}`;
            if(idx === 0) out += `                if ${phaseVar} < ${(1 / count).toFixed(6)}\n                    ${line}\n`;
            else if(idx < count - 1) out += `                else if ${phaseVar} < ${((idx + 1) / count).toFixed(6)}\n                    ${line}\n`;
            else out += `                else\n                    ${line}\n`;
        });
        out += `                endif\n`;
        return out;
    }

    function autoNumber(v, fallback = 0) {
        const n = Number(v);
        return Number.isFinite(n) ? n : fallback;
    }

    function isAutoTruthy(v) {
        return Math.abs(autoNumber(v, 0)) > 1e-9;
    }

    function mod(a, b) {
        if(b === 0) return 0;
        return ((a % b) + b) % b;
    }

    function saturate(v) {
        return clamp(v, 0, 1);
    }

    function repeat(v, period = 1) {
        if(period === 0) return 0;
        return fract(v / period) * period;
    }

    function invlerp(a, b, v) {
        if(a === b) return 0;
        return (v - a) / (b - a);
    }

    function pulse(edge0, edge1, x) {
        return step(edge0, x) - step(edge1, x);
    }

    function select(cond, whenTrue, whenFalse) {
        return isAutoTruthy(cond) ? whenTrue : whenFalse;
    }

    function bezier3(a, b, c, t) {
        return mix(mix(a, b, t), mix(b, c, t), t);
    }

    function bezier4(a, b, c, d, t) {
        return mix(bezier3(a, b, c, t), bezier3(b, c, d, t), t);
    }

    const AUTO_FUNCTION_CONSTANTS = {
        PI: Math.PI,
        TAU: Math.PI * 2,
        E: Math.E
    };

    const AUTO_FUNCTION_LIBRARY = {
        sin: Math.sin,
        cos: Math.cos,
        tan: Math.tan,
        asin: Math.asin,
        acos: Math.acos,
        atan: Math.atan,
        atan2: Math.atan2,
        abs: Math.abs,
        min: Math.min,
        max: Math.max,
        pow: Math.pow,
        sqrt: Math.sqrt,
        cbrt: Math.cbrt,
        log: Math.log,
        log2: Math.log2,
        exp: Math.exp,
        floor: Math.floor,
        ceil: Math.ceil,
        round: Math.round,
        sign: Math.sign,
        clamp,
        fract,
        mix,
        lerp: mix,
        step,
        smoothstep,
        saw,
        tri,
        pingpong,
        sin01,
        cos01,
        mod,
        repeat,
        saturate,
        invlerp,
        pulse,
        select,
        bezier3,
        bezier4
    };

    AUTO_FUNCTION_ALLOWED_IDENTIFIERS.clear();
    AUTO_FUNCTION_ALLOWED_IDENTIFIERS.add('t');
    Object.keys(AUTO_FUNCTION_CONSTANTS).forEach((key) => AUTO_FUNCTION_ALLOWED_IDENTIFIERS.add(key));
    Object.keys(AUTO_FUNCTION_LIBRARY).forEach((key) => AUTO_FUNCTION_ALLOWED_IDENTIFIERS.add(key));

    function tokenizeAutoExpression(source) {
        const tokens = [];
        let i = 0;
        while(i < source.length) {
            const ch = source[i];
            if(/\s/.test(ch)) {
                i++;
                continue;
            }
            if(/[0-9.]/.test(ch)) {
                const start = i;
                let hasDot = ch === '.';
                i++;
                while(i < source.length) {
                    const c = source[i];
                    if(/[0-9]/.test(c)) {
                        i++;
                        continue;
                    }
                    if(c === '.' && !hasDot) {
                        hasDot = true;
                        i++;
                        continue;
                    }
                    if((c === 'e' || c === 'E') && i + 1 < source.length) {
                        const next = source[i + 1];
                        if(/[+\-0-9]/.test(next)) {
                            i += 2;
                            while(i < source.length && /[0-9]/.test(source[i])) i++;
                            continue;
                        }
                    }
                    break;
                }
                const value = Number(source.slice(start, i));
                if(!Number.isFinite(value)) throw new Error(`数字无效: ${source.slice(start, i)}`);
                tokens.push({ type: 'number', value });
                continue;
            }
            if(/[A-Za-z_]/.test(ch)) {
                const start = i;
                i++;
                while(i < source.length && /[A-Za-z0-9_]/.test(source[i])) i++;
                tokens.push({ type: 'identifier', value: source.slice(start, i) });
                continue;
            }
            const two = source.slice(i, i + 2);
            if(['<=', '>=', '==', '!=', '&&', '||'].includes(two)) {
                tokens.push({ type: 'operator', value: two });
                i += 2;
                continue;
            }
            if('+-*/%(),?:!<>'.includes(ch)) {
                tokens.push({ type: 'operator', value: ch });
                i++;
                continue;
            }
            throw new Error(`不支持的字符: ${ch}`);
        }
        tokens.push({ type: 'eof', value: '' });
        return tokens;
    }

    function compileAutoExpression(expr) {
        const source = (expr || '').trim();
        if(!source) return null;

        const tokens = tokenizeAutoExpression(source);
        let index = 0;

        const peek = () => tokens[index];
        const consume = () => tokens[index++];
        const match = (...values) => {
            const token = peek();
            if(token && values.includes(token.value)) {
                index++;
                return true;
            }
            return false;
        };
        const expect = (value) => {
            if(!match(value)) throw new Error(`缺少 ${value}`);
        };

        const parsePrimary = () => {
            const token = peek();
            if(token.type === 'number') {
                consume();
                return () => token.value;
            }
            if(token.type === 'identifier') {
                consume();
                const name = token.value;
                if(!AUTO_FUNCTION_ALLOWED_IDENTIFIERS.has(name)) throw new Error(`表达式包含未授权标识符: ${name}`);
                if(match('(')) {
                    const args = [];
                    if(!match(')')) {
                        do {
                            args.push(parseConditional());
                        } while(match(','));
                        expect(')');
                    }
                    if(!(name in AUTO_FUNCTION_LIBRARY)) throw new Error(`表达式调用了未知函数: ${name}`);
                    return (ctx) => autoNumber(AUTO_FUNCTION_LIBRARY[name](...args.map(fn => fn(ctx))), 0);
                }
                if(name === 't') return (ctx) => autoNumber(ctx.t, 0);
                if(name in AUTO_FUNCTION_CONSTANTS) return () => AUTO_FUNCTION_CONSTANTS[name];
                throw new Error(`不允许直接读取的标识符: ${name}`);
            }
            if(match('(')) {
                const node = parseConditional();
                expect(')');
                return node;
            }
            throw new Error(`无法解析表达式片段: ${token.value || 'EOF'}`);
        };

        const parseUnary = () => {
            if(match('+')) {
                const right = parseUnary();
                return (ctx) => autoNumber(right(ctx), 0);
            }
            if(match('-')) {
                const right = parseUnary();
                return (ctx) => -autoNumber(right(ctx), 0);
            }
            if(match('!')) {
                const right = parseUnary();
                return (ctx) => isAutoTruthy(right(ctx)) ? 0 : 1;
            }
            return parsePrimary();
        };

        const buildBinaryParser = (nextParser, operators) => () => {
            let left = nextParser();
            while(operators.includes(peek().value)) {
                const op = consume().value;
                const prevLeft = left;
                const right = nextParser();
                if(op === '*') left = (ctx) => autoNumber(prevLeft(ctx), 0) * autoNumber(right(ctx), 0);
                else if(op === '/') left = (ctx) => {
                    const denom = autoNumber(right(ctx), 0);
                    return denom === 0 ? 0 : autoNumber(prevLeft(ctx), 0) / denom;
                };
                else if(op === '%') left = (ctx) => mod(autoNumber(prevLeft(ctx), 0), autoNumber(right(ctx), 0));
                else if(op === '+') left = (ctx) => autoNumber(prevLeft(ctx), 0) + autoNumber(right(ctx), 0);
                else if(op === '-') left = (ctx) => autoNumber(prevLeft(ctx), 0) - autoNumber(right(ctx), 0);
                else if(op === '<') left = (ctx) => autoNumber(prevLeft(ctx), 0) < autoNumber(right(ctx), 0) ? 1 : 0;
                else if(op === '<=') left = (ctx) => autoNumber(prevLeft(ctx), 0) <= autoNumber(right(ctx), 0) ? 1 : 0;
                else if(op === '>') left = (ctx) => autoNumber(prevLeft(ctx), 0) > autoNumber(right(ctx), 0) ? 1 : 0;
                else if(op === '>=') left = (ctx) => autoNumber(prevLeft(ctx), 0) >= autoNumber(right(ctx), 0) ? 1 : 0;
                else if(op === '==') left = (ctx) => autoNumber(prevLeft(ctx), 0) === autoNumber(right(ctx), 0) ? 1 : 0;
                else if(op === '!=') left = (ctx) => autoNumber(prevLeft(ctx), 0) !== autoNumber(right(ctx), 0) ? 1 : 0;
                else if(op === '&&') left = (ctx) => (isAutoTruthy(prevLeft(ctx)) && isAutoTruthy(right(ctx))) ? 1 : 0;
                else if(op === '||') left = (ctx) => (isAutoTruthy(prevLeft(ctx)) || isAutoTruthy(right(ctx))) ? 1 : 0;
            }
            return left;
        };

        const parseMulDiv = buildBinaryParser(parseUnary, ['*', '/', '%']);
        const parseAddSub = buildBinaryParser(parseMulDiv, ['+', '-']);
        const parseCompare = buildBinaryParser(parseAddSub, ['<', '<=', '>', '>=']);
        const parseEquality = buildBinaryParser(parseCompare, ['==', '!=']);
        const parseLogicalAnd = buildBinaryParser(parseEquality, ['&&']);
        const parseLogicalOr = buildBinaryParser(parseLogicalAnd, ['||']);
        const parseConditional = () => {
            const condition = parseLogicalOr();
            if(match('?')) {
                const whenTrue = parseConditional();
                expect(':');
                const whenFalse = parseConditional();
                return (ctx) => isAutoTruthy(condition(ctx)) ? whenTrue(ctx) : whenFalse(ctx);
            }
            return condition;
        };

        const evaluator = parseConditional();
        if(peek().type !== 'eof') throw new Error(`组组ı组ʽƬ组: ${peek().value}`);
        return (t) => autoNumber(evaluator({ t }), 0);
    }

    function getAutoSourceMode(component) {
        return component && component.autoSource === 'function' ? 'function' : 'chaos';
    }

    function getAutoPrimaryFunctionLabel(component) {
        return component && component.type === 'joystick' ? '函数 X(t):' : '函数 f(t):';
    }

    function applyAutoEditorState(component) {
        if(!component) return;
        const source = getAutoSourceMode(component);
        const isJoystick = component.type === 'joystick';

        const rowAutoAmpY = document.getElementById('row_auto_amp_y');
        const rowAutoSeedX = document.getElementById('row_auto_seed_x');
        const rowAutoSeedY = document.getElementById('row_auto_seed_y');
        const rowAutoFuncX = document.getElementById('row_auto_func_x');
        const rowAutoFuncY = document.getElementById('row_auto_func_y');
        const lblAutoAmpX = document.getElementById('lbl_auto_amp_x');
        const lblAutoSeedX = document.getElementById('lbl_auto_seed_x');
        const lblAutoFuncX = document.getElementById('lbl_auto_func_x');
        const lblAutoRate = document.getElementById('lbl_auto_rate');
        const autoFuncXInput = document.getElementById('p_auto_func_x');
        const autoFuncYInput = document.getElementById('p_auto_func_y');
        const autoAnimSection = document.getElementById('auto_anim_section');
        const autoFuncHint = document.getElementById('auto_func_hint');

        lblAutoAmpX.innerText = isJoystick ? '幅度范围:' : '自动范围:';
        lblAutoSeedX.innerText = isJoystick ? '随机种子:' : '轨迹种子:';
        lblAutoFuncX.innerText = getAutoPrimaryFunctionLabel(component);
        lblAutoRate.innerText = source === 'function' ? '函数速度:' : '轨迹刷新:';
        if(autoFuncXInput) autoFuncXInput.placeholder = isJoystick ? 'sin(TAU * t)' : 'sin01(t)';
        if(autoFuncYInput) autoFuncYInput.placeholder = 'cos(TAU * t)';
        if(autoAnimSection) autoAnimSection.open = !!component.autoAnimate || source === 'function';

        rowAutoAmpY.style.display = isJoystick ? 'flex' : 'none';
        rowAutoSeedY.style.display = source === 'chaos' && isJoystick ? 'flex' : 'none';
        rowAutoFuncY.style.display = source === 'function' && isJoystick ? 'flex' : 'none';
        rowAutoSeedX.style.display = source === 'chaos' ? 'flex' : 'none';
        rowAutoFuncX.style.display = source === 'function' ? 'flex' : 'none';
        autoFuncHint.style.display = source === 'function' ? 'block' : 'none';
        renderAutoFunctionPreview(component);
    }

    function normalizeAngle(angle) {
        let normalized = Number.isFinite(angle) ? angle % 360 : 0;
        if(normalized < 0) normalized += 360;
        return normalized;
    }

    function formatAngleLabel(angle) {
        let normalized = normalizeAngle(angle);
        let rounded = Math.round(normalized * 100) / 100;
        return `${Number.isInteger(rounded) ? rounded : rounded.toFixed(2).replace(/0+$/, '').replace(/\.$/, '')}°`;
    }

    function componentSupportsPhysics(component) {
        if(!component) return false;
        if(component.type === 'joystick') return true;
        if(component.type === 'slider_h' || component.type === 'slider_v') return component.paramMode === '2';
        return false;
    }

    function getSliderSubdivisionConfig(obj) {
        const subdivisions = clamp(parseInt(obj && obj.sliderSubdivisions, 10) || 1, 1, 8);
        const sideCount = obj && String(obj.paramMode) === '2' ? 2 : 1;
        return {
            subdivisions,
            sideCount,
            totalVars: subdivisions * sideCount,
            segmentSize: 1 / subdivisions
        };
    }

    function isSliderSubdivisionMode(component) {
        return !!(component && component.type && component.type.includes('slider') &&
            (String(component.paramMode) === '1' || String(component.paramMode) === '2') &&
            getSliderSubdivisionConfig(component).subdivisions > 1);
    }

    function getSliderSubdivisionVarIndex(obj, sideIdx, segIdx) {
        const cfg = getSliderSubdivisionConfig(obj);
        return sideIdx * cfg.subdivisions + segIdx;
    }

    function getSliderSubdivisionSideLabel(obj, sideIdx) {
        if(String(obj && obj.paramMode) !== '2') return obj && obj.type === 'slider_v' ? '下到上行程' : '左到右行程';
        if(obj && obj.type === 'slider_v') return sideIdx === 0 ? '下侧' : '上侧';
        return sideIdx === 0 ? '左侧' : '右侧';
    }

    function buildSliderSubdivisionVarName(obj, sideIdx, segIdx, token) {
        let sideTag = 'Travel';
        if(String(obj && obj.paramMode) === '2') {
            if(obj && obj.type === 'slider_v') sideTag = sideIdx === 0 ? 'Bottom' : 'Top';
            else sideTag = sideIdx === 0 ? 'Left' : 'Right';
        }
        return `$${sideTag}_S${segIdx + 1}_${token}`;
    }

    function ensureSliderSubdivisionState(obj, options = {}) {
        if(!obj || !obj.type || !obj.type.includes('slider') || (String(obj.paramMode) !== '1' && String(obj.paramMode) !== '2')) return;
        const cfg = getSliderSubdivisionConfig(obj);
        const oldVars = Array.isArray(obj.vars) ? [...obj.vars] : [];
        const oldDepTargets = Array.isArray(obj.depTargets)
            ? obj.depTargets.map(targets => Array.isArray(targets) ? targets.map(target => ({...target})) : [])
            : [];
        const inferredOldSubdivisions = cfg.sideCount > 0 && oldVars.length >= cfg.sideCount && oldVars.length % cfg.sideCount === 0
            ? oldVars.length / cfg.sideCount
            : 1;
        const oldSubdivisions = clamp(parseInt(options.previousSubdivisions, 10) || inferredOldSubdivisions || 1, 1, 8);
        const resetNames = options.resetNames === true;
        let nameToken = '';
        const nextName = (sideIdx, segIdx) => {
            if(!nameToken) nameToken = nextUniqueToken();
            return buildSliderSubdivisionVarName(obj, sideIdx, segIdx, nameToken);
        };

        obj.sliderSubdivisions = cfg.subdivisions;
        obj.vars = new Array(cfg.totalVars);
        obj.depTargets = new Array(cfg.totalVars);
        for(let sideIdx = 0; sideIdx < cfg.sideCount; sideIdx++) {
            for(let segIdx = 0; segIdx < cfg.subdivisions; segIdx++) {
                const newIdx = getSliderSubdivisionVarIndex(obj, sideIdx, segIdx);
                const sourceIdx = sideIdx * oldSubdivisions + segIdx;
                const hasExistingSegment = segIdx < oldSubdivisions;
                const existingVar = hasExistingSegment ? oldVars[sourceIdx] : undefined;
                obj.vars[newIdx] = !resetNames && existingVar !== undefined ? existingVar : nextName(sideIdx, segIdx);
                obj.depTargets[newIdx] = hasExistingSegment && Array.isArray(oldDepTargets[sourceIdx])
                    ? oldDepTargets[sourceIdx].map(target => ({...target}))
                    : [];
            }
        }
    }

    function getSliderSubdivisionPressure(obj, value, sideIdx) {
        const current = clamp(Number(value) || 0, 0, 1);
        if(String(obj && obj.paramMode) !== '2') return current;
        return sideIdx === 0 ? clamp((0.5 - current) * 2, 0, 1) : clamp((current - 0.5) * 2, 0, 1);
    }

    function getSliderSubdivisionValues(obj, value) {
        const cfg = getSliderSubdivisionConfig(obj);
        const values = new Array(cfg.totalVars).fill(0);
        for(let sideIdx = 0; sideIdx < cfg.sideCount; sideIdx++) {
            const pressure = getSliderSubdivisionPressure(obj, value, sideIdx);
            for(let segIdx = 0; segIdx < cfg.subdivisions; segIdx++) {
                const flatIdx = getSliderSubdivisionVarIndex(obj, sideIdx, segIdx);
                const lowerBound = segIdx * cfg.segmentSize;
                values[flatIdx] = clamp((pressure - lowerBound) * cfg.subdivisions, 0, 1);
            }
        }
        return values;
    }

    function getJoystickConfig(obj) {
        const directionCount = clamp(parseInt(obj.joystickDirectionCount, 10) || 4, 3, 32);
        const subdivisions = clamp(parseInt(obj.joystickSubdivisions, 10) || 1, 1, 8);
        const angleOffset = normalizeAngle(parseFloat(obj.joystickAngleOffset) || 0);
        return {
            directionCount,
            subdivisions,
            angleOffset,
            angleStep: 360 / directionCount,
            segmentSize: 1 / subdivisions
        };
    }

    function getJoystickDirectionAngle(obj, dirIdx) {
        const cfg = getJoystickConfig(obj);
        return normalizeAngle(cfg.angleOffset + dirIdx * cfg.angleStep);
    }

    function getJoystickDirectionVarIndex(obj, dirIdx, segIdx) {
        const cfg = getJoystickConfig(obj);
        return dirIdx * cfg.subdivisions + segIdx;
    }

    function getJoystickDirectionAnchorVector(obj, dirIdx) {
        const angleRad = getJoystickDirectionAngle(obj, dirIdx) * Math.PI / 180;
        const dirX = Math.sin(angleRad);
        const dirY = Math.cos(angleRad);
        const edgeScale = 1 / Math.max(0.000001, Math.max(Math.abs(dirX), Math.abs(dirY)));
        return clampJoystickVectorToRoundedBounds(obj, dirX * edgeScale, dirY * edgeScale);
    }

    function getJoystickDirectionPressureValue(obj, flatValues, dirIdx) {
        const cfg = getJoystickConfig(obj);
        let sum = 0;
        for(let segIdx = 0; segIdx < cfg.subdivisions; segIdx++) {
            const flatIdx = getJoystickDirectionVarIndex(obj, dirIdx, segIdx);
            sum += clamp(Number(flatValues && flatValues[flatIdx]) || 0, 0, 1);
        }
        return clamp(sum / cfg.subdivisions, 0, 1);
    }

    function writeJoystickDirectionPressureValue(obj, target, dirIdx, pressure) {
        const cfg = getJoystickConfig(obj);
        const p = clamp(Number(pressure) || 0, 0, 1);
        for(let segIdx = 0; segIdx < cfg.subdivisions; segIdx++) {
            const flatIdx = getJoystickDirectionVarIndex(obj, dirIdx, segIdx);
            const lowerBound = segIdx * cfg.segmentSize;
            target[flatIdx] = clamp((p - lowerBound) * cfg.subdivisions, 0, 1);
        }
    }

    function getJoystickDirectionalBlendWeights(obj, vector) {
        const cfg = getJoystickConfig(obj);
        const weights = new Array(cfg.directionCount).fill(0);
        const vx = Number(vector && vector.x) || 0;
        const vy = Number(vector && vector.y) || 0;
        if(Math.abs(vx) < 0.000001 && Math.abs(vy) < 0.000001) return weights;
        const anchors = new Array(cfg.directionCount);
        for(let dirIdx = 0; dirIdx < cfg.directionCount; dirIdx++) {
            anchors[dirIdx] = getJoystickDirectionAnchorVector(obj, dirIdx);
        }
        const eps = 0.000001;
        for(let dirIdx = 0; dirIdx < cfg.directionCount; dirIdx++) {
            const nextIdx = (dirIdx + 1) % cfg.directionCount;
            const a = anchors[dirIdx];
            const b = anchors[nextIdx];
            const crossA = a.x * vy - a.y * vx;
            const crossB = vx * b.y - vy * b.x;
            if(crossA <= eps && crossB <= eps) {
                const den = a.x * b.y - a.y * b.x;
                if(Math.abs(den) <= eps) break;
                let w0 = (vx * b.y - vy * b.x) / den;
                let w1 = (a.x * vy - a.y * vx) / den;
                w0 = Math.max(0, w0);
                w1 = Math.max(0, w1);
                weights[dirIdx] = clamp(w0, 0, 1);
                weights[nextIdx] = clamp(w1, 0, 1);
                return weights;
            }
        }
        let bestIdx = 0;
        let bestProj = -Infinity;
        for(let dirIdx = 0; dirIdx < cfg.directionCount; dirIdx++) {
            const anchor = anchors[dirIdx];
            const proj = vx * anchor.x + vy * anchor.y;
            if(proj > bestProj) {
                bestProj = proj;
                bestIdx = dirIdx;
            }
        }
        weights[bestIdx] = clamp(Math.max(Math.abs(vx), Math.abs(vy)), 0, 1);
        return weights;
    }

    function buildJoystickDirectionVarName(ts, angle, segIdx, subdivisions) {
        const angleTag = String(Math.round(normalizeAngle(angle))).padStart(3, '0');
        return subdivisions === 1 ? `$Dir_${angleTag}_${ts}` : `$Dir_${angleTag}_S${segIdx + 1}_${ts}`;
    }

    function sanitizeIniVarToken(value, fallback = '') {
        const text = String(value == null ? '' : value).replace(/[\r\n\t]+/g, ' ').trim();
        if(!text) return fallback;
        const match = text.match(/\$?[A-Za-z_][A-Za-z0-9_\.]*/);
        if(!match) return fallback;
        const raw = match[0];
        return raw.startsWith('$') ? raw : '$' + raw;
    }

    function splitVarStr(str) {
        if(!str || !str.trim()) return [];
        return str.split(',').map(s => s.trim()).filter(s => s && s.trim());
    }

    function normalizeLinkedSlaveActionList(actions) {
        if(!Array.isArray(actions)) return [];
        return actions.map(action => ({
            var: sanitizeIniVarToken(action && action.var, ''),
            value: Number.isFinite(Number(action && action.value)) ? Number(action.value) : 0
        }));
    }

    function getLinkedSlaveConfiguredActions(link, phase = 'enter') {
        if(!link) return [];
        const key = phase === 'leave' ? 'leaveActions' : 'enterActions';
        const actions = Array.isArray(link[key]) ? link[key] : [];
        return actions.filter(action => action && action.var && String(action.var).trim());
    }

    function hasLinkedSlaveConfiguredActions(link) {
        return getLinkedSlaveConfiguredActions(link, 'enter').length > 0 ||
            getLinkedSlaveConfiguredActions(link, 'leave').length > 0;
    }

    function hasLinkedSlaveActionSlots(link) {
        return !!(link && (
            (Array.isArray(link.enterActions) && link.enterActions.length > 0) ||
            (Array.isArray(link.leaveActions) && link.leaveActions.length > 0)
        ));
    }

    function hasRangeTriggerConfiguredActions(trigger) {
        if(!trigger) return false;
        const enter = Array.isArray(trigger.enterActions) ? trigger.enterActions : [];
        const leave = Array.isArray(trigger.leaveActions) ? trigger.leaveActions : [];
        return enter.filter(a => a && a.var && String(a.var).trim()).length > 0 ||
            leave.filter(a => a && a.var && String(a.var).trim()).length > 0;
    }

    function hasRangeTriggerActionSlots(trigger) {
        return !!(trigger && (
            (Array.isArray(trigger.enterActions) && trigger.enterActions.length > 0) ||
            (Array.isArray(trigger.leaveActions) && trigger.leaveActions.length > 0)
        ));
    }

    function buildLinkedSlaveActionINI(actions, indent = '        ') {
        return actions.map(action => `${indent}${action.var} = ${autoNumber(action.value, 0)}\n`).join('');
    }

    function buildDependencyTriggerStateVar(compIdx, paramIdx, targetIdx) {
        return `$dep_state_${compIdx}_${paramIdx}_${targetIdx}`;
    }

    function buildMergedDependencyTriggerStateVar(compIdx, mergeIdx) {
        return `$dep_state_${compIdx}_merge_${mergeIdx}`;
    }

    function buildGridDependencyTriggerStateVar(compIdx, gridIdx, targetIdx) {
        return `$grid_dep_state_${compIdx}_${gridIdx}_${targetIdx}`;
    }

    function getComponentDependencyTriggerSpecs(component, compIdx = components.indexOf(component)) {
        const specs = [];
        if(!component || compIdx < 0) return specs;

        if(isSliderSubdivisionMode(component)) {
            const cfg = getSliderSubdivisionConfig(component);
            for(let flatIdx = 0; flatIdx < cfg.totalVars; flatIdx++) {
                const targets = Array.isArray(component.depTargets && component.depTargets[flatIdx])
                    ? component.depTargets[flatIdx]
                    : [];
                targets.forEach((target, targetIdx) => {
                    const targetVar = sanitizeIniVarToken(target && target.var, '');
                    if(!targetVar) return;
                    const inverted = !!(target && target.invert);
                    specs.push({
                        stateVar: buildDependencyTriggerStateVar(compIdx, flatIdx, targetIdx),
                        targetVar,
                        trueValue: inverted ? 0 : 1,
                        falseValue: inverted ? 1 : 0,
                        useElse: !!(target && target.else),
                        triggerKind: 'slider-segment',
                        paramIndex: flatIdx
                    });
                });
            }
            return specs;
        }

        if(component.type.includes('slider') && component.paramMode === '2') {
            const mergedTargets = new Map();
            (Array.isArray(component.depTargets) ? component.depTargets : []).forEach((targets, vi) => {
                if(!Array.isArray(targets)) return;
                targets.forEach(target => {
                    const targetVar = sanitizeIniVarToken(target && target.var, '');
                    if(!targetVar) return;
                    if(!mergedTargets.has(targetVar)) {
                        mergedTargets.set(targetVar, {
                            targetVar,
                            inverted: !!(target && target.invert),
                            useElse: !!(target && target.else),
                            directions: []
                        });
                    }
                    const merged = mergedTargets.get(targetVar);
                    merged.directions.push(vi);
                    if(target && target.else) merged.useElse = true;
                    if(target && target.invert) merged.inverted = true;
                });
            });
            let mergeIdx = 0;
            mergedTargets.forEach((merged) => {
                specs.push({
                    stateVar: buildMergedDependencyTriggerStateVar(compIdx, mergeIdx++),
                    targetVar: merged.targetVar,
                    trueValue: merged.inverted ? 0 : 1,
                    falseValue: merged.inverted ? 1 : 0,
                    useElse: merged.useElse,
                    triggerKind: 'bidir',
                    directions: merged.directions.slice()
                });
            });
            return specs;
        }

        if(component.type.includes('slider') && component.paramMode === '3') {
            const gridSteps = component.gridSteps || 3;
            const gridDepTargets = Array.isArray(component.gridDepTargets) ? component.gridDepTargets : [];
            const interval = 1.0 / Math.max(1, gridSteps - 1);
            for(let g = 0; g < gridSteps; g++) {
                const targets = Array.isArray(gridDepTargets[g]) ? gridDepTargets[g] : [];
                const thresholdLow = Math.max(0, g * interval - interval * 0.5);
                const thresholdHigh = Math.min(1, g * interval + interval * 0.5);
                targets.forEach((target, targetIdx) => {
                    const targetVar = sanitizeIniVarToken(target && target.var, '');
                    if(!targetVar) return;
                    const inverted = !!(target && target.invert);
                    const useElse = target && typeof target.else !== 'undefined' ? !!target.else : true;
                    specs.push({
                        stateVar: buildGridDependencyTriggerStateVar(compIdx, g, targetIdx),
                        targetVar,
                        trueValue: inverted ? 0 : 1,
                        falseValue: inverted ? 1 : 0,
                        useElse,
                        triggerKind: 'grid',
                        gridIndex: g,
                        gridSteps,
                        thresholdLow,
                        thresholdHigh
                    });
                });
            }
            return specs;
        }

        (Array.isArray(component.depTargets) ? component.depTargets : []).forEach((targets, paramIdx) => {
            if(!Array.isArray(targets)) return;
            targets.forEach((target, targetIdx) => {
                const targetVar = sanitizeIniVarToken(target && target.var, '');
                if(!targetVar) return;
                const inverted = !!(target && target.invert);
                specs.push({
                    stateVar: buildDependencyTriggerStateVar(compIdx, paramIdx, targetIdx),
                    targetVar,
                    trueValue: inverted ? 0 : 1,
                    falseValue: inverted ? 1 : 0,
                    useElse: !!(target && target.else),
                    triggerKind: component.type === 'joystick'
                        ? (component.paramMode === '4' ? 'joy-dir' : 'joy-axis')
                        : 'value-threshold',
                    paramIndex: paramIdx
                });
            });
        });
        return specs;
    }

    function ensureJoystickDirectionState(obj, options = {}) {
        if(!obj || obj.type !== 'joystick' || obj.paramMode !== '4') return;

        const { resetNames = false, legacyRemap = null } = options;
        const ts = nextUniqueToken();
        const cfg = getJoystickConfig(obj);
        const totalVars = cfg.directionCount * cfg.subdivisions;
        const oldVars = Array.isArray(obj.vars) ? [...obj.vars] : [];
        const oldMaxVals = Array.isArray(obj.maxVals) ? [...obj.maxVals] : [];
        const oldDepTargets = Array.isArray(obj.depTargets) ? obj.depTargets.map(targets => Array.isArray(targets) ? [...targets] : []) : [];

        obj.vars = new Array(totalVars);
        obj.maxVals = new Array(totalVars);
        obj.depTargets = new Array(totalVars);
        obj.defVals = [];

        for(let dirIdx = 0; dirIdx < cfg.directionCount; dirIdx++) {
            const angle = getJoystickDirectionAngle(obj, dirIdx);
            for(let segIdx = 0; segIdx < cfg.subdivisions; segIdx++) {
                const newIdx = dirIdx * cfg.subdivisions + segIdx;
                const sourceIdx = Array.isArray(legacyRemap) && legacyRemap[newIdx] !== undefined ? legacyRemap[newIdx] : newIdx;
                const existingVar = oldVars[sourceIdx];
                obj.vars[newIdx] = (!resetNames && existingVar !== undefined && existingVar !== '') ? existingVar : buildJoystickDirectionVarName(ts, angle, segIdx, cfg.subdivisions);
                obj.maxVals[newIdx] = oldMaxVals[sourceIdx] !== undefined ? Math.max(0.1, parseFloat(oldMaxVals[sourceIdx]) || 1) : 1;
                obj.depTargets[newIdx] = Array.isArray(oldDepTargets[sourceIdx]) ? oldDepTargets[sourceIdx].map(target => ({...target})) : [];
            }
        }
    }

    const gridTargetEditorState = new WeakMap();

    function isSliderGridMode(component) {
        return !!(component && component.type && component.type.includes('slider') && String(component.paramMode) === '3');
    }

    function clampGridStepCount(value, fallback = 3) {
        const parsed = parseInt(value, 10);
        return Math.max(1, Number.isFinite(parsed) ? parsed : fallback);
    }

    function getSliderGridMainVars(component) {
        if(!isSliderGridMode(component)) return [];
        const rawVars = Array.isArray(component.vars)
            ? component.vars
            : component.vars == null ? [] : [component.vars];
        return rawVars
            .map(v => sanitizeIniVarToken(v, ''))
            .filter(v => v && v.trim());
    }

    function getSliderGridPrimaryVar(component) {
        const vars = getSliderGridMainVars(component);
        return vars.length > 0 ? vars[0].trim() : '';
    }

    function getSliderGridValueStart(component) {
        const value = Number(component && component.gridValueStart);
        return Number.isFinite(value) ? value : 0;
    }

    function getSliderGridValueStep(component) {
        const value = Number(component && component.gridValueStep);
        return Number.isFinite(value) && Math.abs(value) > 1e-12 ? value : 1;
    }

    function cleanSliderGridNumber(value) {
        if(!Number.isFinite(value)) return 0;
        return Number.parseFloat(value.toPrecision(12));
    }

    function getSliderGridOutputValue(component, index) {
        const gridSteps = clampGridStepCount(component && component.gridSteps, 3);
        const safeIndex = clamp(Math.round(Number(index) || 0), 0, Math.max(0, gridSteps - 1));
        return cleanSliderGridNumber(getSliderGridValueStart(component) + safeIndex * getSliderGridValueStep(component));
    }

    function getSliderGridIndexFromOutputValue(component, outputValue) {
        const start = getSliderGridValueStart(component);
        const step = getSliderGridValueStep(component);
        const rawIndex = (Number(outputValue) - start) / step;
        return clamp(Math.round(Number.isFinite(rawIndex) ? rawIndex : 0), 0, Math.max(0, clampGridStepCount(component && component.gridSteps, 3) - 1));
    }

    function formatSliderGridNumber(value) {
        const cleaned = cleanSliderGridNumber(value);
        if(Number.isInteger(cleaned)) return String(cleaned);
        return cleaned.toFixed(8).replace(/0+$/, '').replace(/\.$/, '');
    }

    function getSliderGridValueSummary(component) {
        const gridSteps = clampGridStepCount(component && component.gridSteps, 3);
        const values = gridSteps <= 7
            ? Array.from({ length: gridSteps }, (_, index) => index)
            : [0, 1, 2, null, gridSteps - 3, gridSteps - 2, gridSteps - 1];
        const preview = values.map(index => index === null ? '…' : formatSliderGridNumber(getSliderGridOutputValue(component, index))).join(' / ');
        return `${preview}（步长 ${formatSliderGridNumber(getSliderGridValueStep(component))}，共 ${gridSteps} 档）`;
    }

    function hasGridDepTargets(component) {
        return Array.isArray(component && component.gridDepTargets) &&
            component.gridDepTargets.some(targets => Array.isArray(targets) && targets.length > 0);
    }

    function isGridTargetEditorOpen(component) {
        if(!component) return false;
        if(gridTargetEditorState.has(component)) return !!gridTargetEditorState.get(component);
        return hasGridDepTargets(component);
    }

    function setGridTargetEditorOpen(component, isOpen) {
        if(!component) return;
        gridTargetEditorState.set(component, !!isOpen);
    }

    function normalizeSliderGridState(component, options = {}) {
        if(!isSliderGridMode(component)) return;

        const { createDefaultVar = false, resetGridTargets = false, resetMainVars = false } = options;
        const rawVars = resetMainVars
            ? []
            : (Array.isArray(component.vars) ? component.vars : component.vars == null ? [] : [component.vars]);
        const normalizedVars = rawVars.map(v => sanitizeIniVarToken(v, ''));
        const gridSteps = clampGridStepCount(component.gridSteps, 3);
        const fallbackDefault = Number.isFinite(Number(component.initialValue)) ? Number(component.initialValue) : 0;
        const currentDefault = Array.isArray(component.defVals) && component.defVals.length > 0
            ? Number(component.defVals[0])
            : fallbackDefault;

        component.gridSteps = gridSteps;
        component.gridValueStart = getSliderGridValueStart(component);
        component.gridValueStep = getSliderGridValueStep(component);
        component.paramMode = '3';
        component.vars = normalizedVars.length > 0 ? normalizedVars : [createDefaultVar ? `$Grid_${nextUniqueToken()}` : ''];
        if(createDefaultVar && component.vars.every(v => !v)) component.vars[0] = `$Grid_${nextUniqueToken()}`;
        component.maxVals = [Math.max(1, gridSteps - 1)];
        component.defVals = [clamp(Math.round(Number.isFinite(currentDefault) ? currentDefault : fallbackDefault), 0, Math.max(0, gridSteps - 1))];

        if(resetGridTargets || !Array.isArray(component.gridDepTargets)) component.gridDepTargets = [];
        component.gridDepTargets = component.gridDepTargets.map(targets => Array.isArray(targets) ? targets : []);
        while(component.gridDepTargets.length < gridSteps) component.gridDepTargets.push([]);
        while(component.gridDepTargets.length > gridSteps) component.gridDepTargets.pop();
    }

    // 跟随鼠标时的偏移比例：以组件宽/高为单位，表示组件有多大部分位于指针“之前”。
    // 0.5 = 中心对准指针；x=1 组件在指针左侧，x=0 在右侧；y=1 在上方，y=0 在下方。
    // 支持任意小数与超出 0~1 的值，可表达斜对角或与指针留出间距。
    // ---- 模型区域拖拽（滑条/摇杆）：命中模型指定区域时视为抓住手柄 ----
    const DEFAULT_ZONE_DRAG_VAR = '$ssmtdrag_ui_zone';

    function componentSupportsZoneDrag(component) {
        return !!component && !!component.type && (component.type.includes('slider') || component.type === 'joystick');
    }

    function componentUsesZoneDrag(component) {
        return componentSupportsZoneDrag(component) && component.zoneDragEnabled === true;
    }

    function getZoneDragVarName(component) {
        return sanitizeIniVarToken(component && component.zoneDragVar, DEFAULT_ZONE_DRAG_VAR);
    }

    function getZoneDragZoneId(component) {
        const value = Math.floor(Number(component && component.zoneDragZoneId));
        return Number.isFinite(value) && value > 0 ? value : 0;
    }

    // 拖拽侧命名空间后缀：从用户填写的“区域变量名”推导（$ssmtdrag_ui_zone_xxx → xxx）。
    // 拖拽侧契约变量（$ssmtdrag_drag_enabled_{ns} / $ssmtdrag_modifier_down_{ns} /
    // $ssmtdrag_lmb_down_{ns} / $ssmtdrag_x_down_{ns}）与面板侧锁存变量共用此后缀。
    // 变量名不带 ssmtdrag_ui_zone 前缀（或恰为 $ssmtdrag_ui_zone）时后缀为空。
    function getZoneDragNamespace(component) {
        const base = 'ssmtdrag_ui_zone';
        const varName = getZoneDragVarName(component);
        const name = varName.startsWith('$') ? varName.slice(1) : varName;
        if(name === base) return '';
        if(!name.startsWith(base + '_') && !name.startsWith(base + '.')) return '';
        return name.slice(base.length).replace(/^[_.]+/, '').replace(/[^A-Za-z0-9_]/g, '_');
    }

    // 面板侧锁存变量（每组件一个）：绑定后锁存到松开，移出区域不丢。
    // 命名刻意避开拖拽侧重写范围（$ssmtdrag_ui_detected_* / $ssmtdrag_ui_zone_*）。
    function getZoneDragLatchVarName(component, index) {
        const ns = getZoneDragNamespace(component);
        return `$ssmtdrag_zone_bound${ns ? `_${ns}` : ''}_${index + 1}`;
    }

    function getFollowCursorOffsets(component) {
        const clampOffset = value => {
            const num = Number(value);
            return Number.isFinite(num) ? Math.min(10, Math.max(-10, num)) : 0.5;
        };
        return {
            x: clampOffset(component && component.followOffsetX),
            y: clampOffset(component && component.followOffsetY)
        };
    }

    // 生成跟随鼠标的 INI 位置表达式：cursor - size * ratio 的简洁形式。
    function formatFollowCursorExpr(cursorExpr, sizeExpr, fraction) {
        const f = Math.round((Number(fraction) || 0) * 10000) / 10000;
        if(f === 0) return cursorExpr;
        if(f === 1) return `${cursorExpr} - ${sizeExpr}`;
        if(f === -1) return `${cursorExpr} + ${sizeExpr}`;
        if(f < 0) return `${cursorExpr} + ${sizeExpr}*${-f}`;
        return `${cursorExpr} - ${sizeExpr}*${f}`;
    }

    function normalizeComponentState(component, options = {}) {
        const legacyDirectionStateMissing =
            typeof component.joystickDirectionCount === 'undefined' ||
            typeof component.joystickSubdivisions === 'undefined' ||
            typeof component.joystickAngleOffset === 'undefined';

        if(!component.paths) component.paths = {bg: ''};
        Object.keys(component.paths).forEach(key => {
            component.paths[key] = normalizeAssetPath(component.paths[key]);
        });
        if(Array.isArray(component.frames)) {
            component.frames.forEach(frame => {
                if(frame && frame.path) frame.path = normalizeAssetPath(frame.path);
            });
        }
        if(!component.paths.bg) component.paths.bg = '';
        if(!component.preview) component.preview = {};
        if(!component.embeddedAssets) component.embeddedAssets = {};
        if(component.type === 'joystick') {
            if(!component.paths.post && component.paths.post_marker) component.paths.post = component.paths.post_marker;
            if(component.paths.post === LEGACY_ASSET_PATHS.postMarker) component.paths.post = DEFAULT_ASSET_PATHS.postMarker;
            if(!component.preview.post && component.preview.post_marker) component.preview.post = component.preview.post_marker;
            if(!component.embeddedAssets.post && component.embeddedAssets.post_marker) component.embeddedAssets.post = component.embeddedAssets.post_marker;
        }
        if(!Array.isArray(component.vars)) component.vars = component.vars == null ? [] : [component.vars];
        if(component.type === 'toggle' && component.vars.length === 1 && typeof component.vars[0] === 'string' && component.vars[0].includes(',')) {
            component.vars = component.vars[0].split(',').map(s => s.trim());
        }
        component.vars = component.vars.map(v => {
            // 非 toggle 类型支持逗号分隔多变量（逐行添加模式）
            if(component.type !== 'toggle' && typeof v === 'string' && v.includes(',')) {
                return v.split(',').map(p => sanitizeIniVarToken(p.trim(), '')).filter(p => p).join(', ');
            }
            return sanitizeIniVarToken(v, '');
        });
        if(typeof component.pinned === 'undefined') component.pinned = false;
        if(typeof component.bindingEnabled === 'undefined') component.bindingEnabled = true;
        component.followCursor = component.followCursor === true;
        // 旧版 followAnchor 方位迁移为偏移比例
        if(typeof component.followOffsetX === 'undefined' && typeof component.followAnchor === 'string') {
            component.followOffsetX = component.followAnchor === 'left' ? 1 : (component.followAnchor === 'right' ? 0 : 0.5);
            component.followOffsetY = component.followAnchor === 'top' ? 1 : (component.followAnchor === 'bottom' ? 0 : 0.5);
        }
        delete component.followAnchor;
        const followOffsets = getFollowCursorOffsets(component);
        component.followOffsetX = followOffsets.x;
        component.followOffsetY = followOffsets.y;
        component.zoneDragEnabled = component.zoneDragEnabled === true;
        component.zoneDragVar = getZoneDragVarName(component);
        component.zoneDragZoneId = getZoneDragZoneId(component);
        if(typeof component.switchGroup === 'undefined') component.switchGroup = 0;
        if(typeof component.initialValue === 'undefined') component.initialValue = 0;
        if(typeof component.paramMode === 'undefined') component.paramMode = component.type === 'joystick' ? '4' : '1';
        component.paramMode = String(component.paramMode);
        if(!Array.isArray(component.linkedSlaves)) component.linkedSlaves = [];
        // 确保 linkedSlaves 条目有必要的字段。允许“仅区间触发动作”而不绑定目标组件。
        component.linkedSlaves = component.linkedSlaves.filter(l => l && (l.targetId || hasLinkedSlaveConfiguredActions(l) || hasLinkedSlaveActionSlots(l)));
        component.linkedSlaves.forEach(l => {
            if(typeof l.srcMin !== 'number') l.srcMin = 0;
            if(typeof l.srcMax !== 'number') l.srcMax = 0.5;
            if(!l.overflow) l.overflow = 'reset';
            if(!l.splitSide) l.splitSide = 'both';
            if(l.joyAxis === undefined || l.joyAxis === null || l.joyAxis === '') l.joyAxis = 'both';
            if(!l.joyTargetAxis) l.joyTargetAxis = 'x';
            // 摇杆→双向滑条：第二源轴，默认不启用
            if(l.joyAxis2 === undefined) l.joyAxis2 = null;
            // 如果目标为方向摇杆且 joyTargetAxis 仍为 'x'/'y'，修正为 '0'
            if(l.targetId) {
                const tgt = components.find(c => c.id === l.targetId);
                if(tgt && tgt.type === 'joystick' && tgt.paramMode === '4') {
                    if(l.joyTargetAxis === 'x' || l.joyTargetAxis === 'y' || l.joyTargetAxis === 'both') {
                        l.joyTargetAxis = '0';
                    }
                    // 双向滑条→方向摇杆：第二个目标方向
                    if(l.joyTargetAxis2 === undefined) l.joyTargetAxis2 = null;
                    // 方向摇杆默认全范围 [0, 1] 而非 [0, 0.5]
                    if(l.srcMin === 0 && l.srcMax === 0.5) {
                        l.srcMax = 1;
                    }
                }
            }
            // 方向源摇杆→任意目标：joyAxis 为方向索引时默认区间 [0, 1]
            if(component.paramMode === '4' && component.type === 'joystick') {
                const ax = l.joyAxis;
                if(ax !== undefined && ax !== null && ax !== '' && ax !== 'x' && ax !== 'y' && ax !== 'both') {
                    if(l.srcMin === 0 && l.srcMax === 0.5) {
                        l.srcMax = 1;
                    }
                }
            }
            if(typeof l.enabled !== 'boolean') l.enabled = true;
            if(typeof l.postEnabled !== 'boolean') l.postEnabled = false;
            if(typeof l.postRadius !== 'number') l.postRadius = 0.25;
            if(typeof l.srcMinY !== 'number') l.srcMinY = -1;
            if(typeof l.srcMaxY !== 'number') l.srcMaxY = 1;
            // 四边形区域点: 优先使用 regionPoints, 否则从旧字段迁移
            if(!Array.isArray(l.regionPoints) || l.regionPoints.length !== 4) {
                l.regionPoints = [
                    {x: l.srcMin, y: l.srcMaxY},
                    {x: l.srcMax, y: l.srcMaxY},
                    {x: l.srcMax, y: l.srcMinY},
                    {x: l.srcMin, y: l.srcMinY}
                ];
            }
            // 摇杆→摇杆联动: 目标始终双轴
            if(component.type === 'joystick' && l.targetId) {
                const tgt = components.find(c => c.id === l.targetId);
                if(tgt && tgt.type === 'joystick') {
                    l.joyTargetAxis = 'both';
                }
                l.regionMode = getLinkedSlaveEffectiveRegionMode(l, component, tgt);
            } else {
                l.regionMode = getLinkedSlaveEffectiveRegionMode(l, component, null);
            }
            l.enterActions = normalizeLinkedSlaveActionList(l.enterActions);
            l.leaveActions = normalizeLinkedSlaveActionList(l.leaveActions);
        });
        if(!Array.isArray(component.rangeTriggers)) component.rangeTriggers = [];
        // 只保留有配置动作的独立区间触发条目
        component.rangeTriggers = component.rangeTriggers.filter(rt => rt && (hasRangeTriggerConfiguredActions(rt) || hasRangeTriggerActionSlots(rt)));
        component.rangeTriggers.forEach(rt => {
            if(typeof rt.srcMin !== 'number') rt.srcMin = 0;
            if(typeof rt.srcMax !== 'number') rt.srcMax = 0.5;
            if(typeof rt.srcMinY !== 'number') rt.srcMinY = -1;
            if(typeof rt.srcMaxY !== 'number') rt.srcMaxY = 1;
            rt.regionPoints = getRangeTriggerRegionPoints(rt);
            rt.enterActions = normalizeLinkedSlaveActionList(rt.enterActions);
            rt.leaveActions = normalizeLinkedSlaveActionList(rt.leaveActions);
        });
        if(component.handleSize == null || !Number.isFinite(Number(component.handleSize))) component.handleSize = DEFAULT_HS;
        if(component.trackThick == null || !Number.isFinite(Number(component.trackThick))) component.trackThick = DEFAULT_TT;
        if(component.cornerRadius == null || !Number.isFinite(Number(component.cornerRadius))) component.cornerRadius = null;
        if(typeof component.springK === 'undefined') component.springK = 0.05;
        if(typeof component.springD === 'undefined') component.springD = 0.95;
        if(typeof component.physicsProfile === 'undefined') component.physicsProfile = 'normal';
        if(!component.minVals) component.minVals = [];
        if(!component.maxVals) component.maxVals = [];
        if(!component.defVals) component.defVals = [];
        if(!component.depTargets) component.depTargets = [];
        if(!component.gridDepTargets) component.gridDepTargets = [];
        component.depTargets = Array.isArray(component.depTargets) ? component.depTargets.map(targets => Array.isArray(targets) ? targets.map(target => ({
            var: sanitizeIniVarToken(target && target.var, ''),
            invert: !!(target && target.invert),
            else: !!(target && target.else)
        })) : []) : [];
        component.gridDepTargets = Array.isArray(component.gridDepTargets) ? component.gridDepTargets.map(targets => Array.isArray(targets) ? targets.map(target => ({
            var: sanitizeIniVarToken(target && target.var, ''),
            invert: !!(target && target.invert),
            else: target && typeof target.else !== 'undefined' ? !!target.else : true
        })) : []) : [];
        if(typeof component.autoAnimate === 'undefined') component.autoAnimate = false;
        if(typeof component.autoStr === 'undefined') component.autoStr = 0.1;
        if(typeof component.autoSource === 'undefined') component.autoSource = 'chaos';
        if(typeof component.autoAmpX === 'undefined') component.autoAmpX = 1;
        if(typeof component.autoAmpY === 'undefined') component.autoAmpY = 1;
        if(typeof component.autoSeedX === 'undefined') component.autoSeedX = 0.3187;
        if(typeof component.autoSeedY === 'undefined') component.autoSeedY = 0.6123;
        if(typeof component.autoFuncX === 'undefined') component.autoFuncX = component.type === 'joystick' ? 'sin(TAU * t)' : 'sin01(t)';
        if(typeof component.autoFuncY === 'undefined') component.autoFuncY = 'cos(TAU * t)';
        if(component.type === 'joystick' && component.autoFuncX === 'sin01(t)') component.autoFuncX = 'sin(TAU * t)';
        if(typeof component.autoSpeed === 'undefined') component.autoSpeed = 0.015;
        if(typeof component.autoResponse === 'undefined') component.autoResponse = 0.22;
        if(typeof component.autoBounce === 'undefined') component.autoBounce = 0.25;
        if(typeof component.gravity === 'undefined') component.gravity = 0;
        if(typeof component.chaosRate === 'undefined') component.chaosRate = 96;
        if(typeof component.sliderSubdivisions === 'undefined') component.sliderSubdivisions = 1;
        if(typeof component.joystickDirectionCount === 'undefined') component.joystickDirectionCount = 4;
        if(typeof component.joystickSubdivisions === 'undefined') component.joystickSubdivisions = 1;
        if(typeof component.joystickAngleOffset === 'undefined') component.joystickAngleOffset = 0;
        if(typeof component.joystickDefaultX === 'undefined') component.joystickDefaultX = (component.type === 'joystick' && component.defVals[0] !== undefined) ? component.defVals[0] : 0;
        if(typeof component.joystickDefaultY === 'undefined') component.joystickDefaultY = (component.type === 'joystick' && component.defVals[1] !== undefined) ? component.defVals[1] : 0;
        if(component.type && component.type.includes('slider') && component.paramMode === '1') {
            let sliderMax = Number.isFinite(Number(component.maxVals[0])) ? Number(component.maxVals[0]) : 1;
            if(component.minVals[0] === undefined) {
                const legacyNormalizedDefault = clamp(Number((component.defVals && component.defVals[0]) ?? 0), 0, 1);
                component.minVals[0] = 0;
                component.defVals[0] = legacyNormalizedDefault * sliderMax;
            }
            let sliderMin = Number.isFinite(Number(component.minVals[0])) ? Number(component.minVals[0]) : 0;
            if(sliderMax < sliderMin) {
                const temp = sliderMax;
                sliderMax = sliderMin;
                sliderMin = temp;
            }
            component.minVals[0] = sliderMin;
            component.maxVals[0] = sliderMax;
            component.defVals[0] = clampSliderActualValue(component, component.defVals[0]);
        }
        normalizeAnimationState(component);
        if(component.type === 'text') {
            if(component.charSize == null || !Number.isFinite(Number(component.charSize))) component.charSize = 0.03;
            if(component.lineGap == null || !Number.isFinite(Number(component.lineGap))) component.lineGap = 0.0;
            component.triggerCooldownSeconds = Math.max(0, Number(component.triggerCooldownSeconds) || 0);
            component.lifetimeSeconds = Math.max(0, Number(component.lifetimeSeconds) || 0);
            if(typeof component.textVisibilityEnabled === 'undefined') component.textVisibilityEnabled = !!sanitizeIniVarToken(component.visVar, '');
            component.textVisibilityEnabled = component.textVisibilityEnabled === true;
            component.visVar = sanitizeIniVarToken(component.visVar, getDefaultTextVisibilityVar(component));
            component.visDefault = component.visDefault !== false;
            component.textHoverEffect = component.textHoverEffect === true;
            component.textClickVar = sanitizeIniVarToken(component.textClickVar, '');
            delete component.textClickTargets;
            component.valVar = sanitizeIniVarToken(component.valVar, '');
            if(!component.fontFamily) component.fontFamily = "Microsoft YaHei";
            if(!component.fontColor) component.fontColor = "#ffffff";
            if(typeof component.fontBold === 'undefined') component.fontBold = true;
            if(typeof component.fontItalic === 'undefined') component.fontItalic = false;
            if(component.textFlow !== 'vertical') component.textFlow = 'horizontal';
            if(typeof component.colorOverrides === 'undefined') component.colorOverrides = {};
        }
        if(component.type === 'sequence') {
            component.seqVar = sanitizeIniVarToken(component.seqVar, '$State');
            if(!component.frames) component.frames = [];
        }
        if(component.type === 'accum') {
            if(!Array.isArray(component.accumBindings)) component.accumBindings = [];
            component.accumBindings = component.accumBindings.filter(b => b && b.targetId && (typeof b.targetId === 'string') && components.some(c => c.id === b.targetId));
            component.accumBindings.forEach(b => { if(!b.kind) b.kind = 'auto'; });
            const parsedThreshold = Number(component.accumThreshold);
            component.accumThreshold = Number.isFinite(parsedThreshold) && parsedThreshold > 0 ? parsedThreshold : 5;
            if(component.accumDirection !== 'v') component.accumDirection = 'h';
            if(!Array.isArray(component.accumTriggers)) component.accumTriggers = [];
            component.accumTriggers = component.accumTriggers.filter(t => t && t.var && String(t.var).trim()).map(t => ({
                var: sanitizeIniVarToken(t.var, ''),
                value: Number.isFinite(Number(t.value)) ? Number(t.value) : 0
            }));
        }

        const defaultPaths = getDefaultComponentPaths(component.type);
        component.paths = Object.assign({}, defaultPaths, component.paths || {});
        if(component.type === 'static' || component.type === 'sequence' || component.type === 'toggle') {
            delete component.paths.bg;
            delete component.preview.bg;
            delete component.embeddedAssets.bg;
            if(component.resourceOpacity) delete component.resourceOpacity.bg;
        }
        ensureComponentResourceOpacityState(component);

        ensureEmbeddedAssetState(component);

        if(component.type === 'toggle') normalizeToggleState(component);

        if(component.type === 'static' && !component.preview.img) component.preview.img = getPreviewTextureUrl(component, 'img');
        if(component.type === 'sequence' && Array.isArray(component.frames)) {
            component.frames.forEach(frame => {
                if(frame && frame.dataUrl && !frame.preview) frame.preview = frame.dataUrl;
            });
        }
        if(component.type === 'toggle') {
            if(!component.preview.off) component.preview.off = getPreviewTextureUrl(component, 'off');
            if(!component.preview.on) component.preview.on = getPreviewTextureUrl(component, 'on');
            if(!component.preview.prog_off) component.preview.prog_off = getPreviewTextureUrl(component, 'prog_off');
            if(!component.preview.prog_on) component.preview.prog_on = getPreviewTextureUrl(component, 'prog_on');
        } else if(component.type === 'joystick') {
            if(!component.preview.bg) component.preview.bg = getPreviewTextureUrl(component, 'bg');
            if(!component.preview.handle) component.preview.handle = getPreviewTextureUrl(component, 'handle');
            if(!component.preview.post) component.preview.post = getPreviewTextureUrl(component, 'post');
        } else if(component.type !== 'text') {
            if(!component.preview.bg) component.preview.bg = getPreviewTextureUrl(component, 'bg');
            if(!component.preview.handle) component.preview.handle = getPreviewTextureUrl(component, 'handle');
            if(!component.preview.bar_l) component.preview.bar_l = getPreviewTextureUrl(component, 'bar_l');
            if(!component.preview.bar_r) component.preview.bar_r = getPreviewTextureUrl(component, 'bar_r');
        }

        if(component.type === 'joystick' && component.paramMode === '4') {
            const shouldMigrateLegacy = options.migrateLegacyJoystick &&
                legacyDirectionStateMissing &&
                Array.isArray(component.vars) &&
                component.vars.length === 4;
            ensureJoystickDirectionState(component, {
                resetNames: false,
                legacyRemap: shouldMigrateLegacy ? JOYSTICK_LEGACY_DIR_REMAP : null
            });
        }
        if(component.type && component.type.includes('slider') && (component.paramMode === '1' || component.paramMode === '2')) {
            ensureSliderSubdivisionState(component);
        }
        if(isSliderGridMode(component)) normalizeSliderGridState(component);
    }

    function toggleTheme() { root.classList.toggle('light-mode'); }
    
    function updateAspectRatio() {
        const aspectInput = document.getElementById('global_aspect') as HTMLInputElement | null;
        if(!aspectInput || !workArea?.isConnected) return;
        const aspect = parseFloat(aspectInput.value) || 1.777;
        syncGridSnapYWithAspect();
        const design = getWorkAreaDesignSize();
        workArea.style.width = `${design.width}px`;
        workArea.style.height = `${design.height}px`;
        resize();
        updateGridVis();
    }

    function getWorkspaceToolInsets(viewportRect) {
        const insets = { left: 0, right: 0 };
        if(!viewportRect || viewportRect.width <= 1) return insets;
        const midpoint = viewportRect.left + viewportRect.width * 0.5;
        const gap = 12;
        ['settingsWindow', 'animColumn', 'componentPanelWrap', 'resourceWindow'].forEach(id => {
            const element = document.getElementById(id);
            if(!element || window.getComputedStyle(element).display === 'none') return;
            const rect = element.getBoundingClientRect();
            if(rect.width <= 1 || rect.height <= 1) return;
            if(rect.bottom <= viewportRect.top || rect.top >= viewportRect.bottom) return;
            if(rect.right <= viewportRect.left || rect.left >= viewportRect.right) return;
            if(rect.left <= midpoint) {
                insets.left = Math.max(insets.left, Math.min(rect.right, viewportRect.right) - viewportRect.left + gap);
            } else {
                insets.right = Math.max(insets.right, viewportRect.right - Math.max(rect.left, viewportRect.left) + gap);
            }
        });
        const maxSingleInset = viewportRect.width * 0.46;
        insets.left = Math.min(Math.max(0, insets.left), maxSingleInset);
        insets.right = Math.min(Math.max(0, insets.right), maxSingleInset);
        const maxCombinedInset = viewportRect.width * 0.72;
        const combined = insets.left + insets.right;
        if(combined > maxCombinedInset && combined > 0) {
            const factor = maxCombinedInset / combined;
            insets.left *= factor;
            insets.right *= factor;
        }
        return insets;
    }

    function resize() {
        if(!root.isConnected || !workArea?.isConnected) return;
        const design = getWorkAreaDesignSize();
        const targetW = design.width;
        const viewport = document.querySelector('.viewport');
        if(!viewport) return;
        const viewportRect = viewport.getBoundingClientRect();
        const toolInsets = getWorkspaceToolInsets(viewportRect);
        viewport.style.paddingLeft = `${Math.round(toolInsets.left)}px`;
        viewport.style.paddingRight = `${Math.round(toolInsets.right)}px`;
        const padX = 32;
        const padY = 32;
        const usableW = Math.max(120, viewportRect.width - padX - toolInsets.left - toolInsets.right);
        const usableH = Math.max(120, viewportRect.height - padY);
        let scale = Math.min(usableW / targetW, usableH / BASE_HEIGHT);
        if(!Number.isFinite(scale) || scale <= 0) scale = 1;
        scale = Math.max(0.18, Math.min(scale, 1.6));
        workArea.style.transform = '';
        workArea.style.width = `${Math.round(design.width * scale)}px`;
        workArea.style.height = `${Math.round(design.height * scale)}px`;
    }
    window.onresize = resize;
    toggleGridSnapAutoY();
    updateAspectRatio();

    function getGridSnapInputs() {
        const aspectInput = document.getElementById('global_aspect') as HTMLInputElement | null;
        const snapXInput = document.getElementById('grid_snap_x') as HTMLInputElement | null;
        const snapYInput = document.getElementById('grid_snap_y') as HTMLInputElement | null;
        const aspect = parseFloat(aspectInput?.value || '') || 1.777;
        const snapX = parseFloat(snapXInput?.value || '');
        const snapY = parseFloat(snapYInput?.value || '');
        return {
            aspect,
            snapX: Number.isFinite(snapX) ? snapX : 0,
            snapY: Number.isFinite(snapY) ? snapY : 0
        };
    }

    function syncGridSnapYWithAspect() {
        const autoY = document.getElementById('grid_snap_auto_y');
        if(!autoY || !autoY.checked) return;
        const { aspect, snapX } = getGridSnapInputs();
        const nextY = snapX > 0 ? (snapX / aspect) : 0;
        const yInput = document.getElementById('grid_snap_y');
        if(yInput) yInput.value = Math.round(nextY * 100000) / 100000;
    }

    function getGridSnapConfig() {
        const { aspect, snapX, snapY } = getGridSnapInputs();
        if(!snapX || snapX <= 0 || !snapY || snapY <= 0) return { enabled: false, snapX: 0, snapY: 0, cellPxX: 0, cellPxY: 0, cols: 0, rows: 0 };
        const rows = Math.max(1, Math.round(1 / snapY));
        const cols = Math.max(1, Math.round(1 / snapX));
        const design = getWorkAreaDesignSize();
        return {
            enabled: true,
            snapX: Math.round(snapX * 1000000) / 1000000,
            snapY: Math.round(snapY * 1000000) / 1000000,
            cellPxX: design.width * snapX,
            cellPxY: design.height * snapY,
            cols,
            rows
        };
    }

    function safeNum(value, fallback = 0) {
        if(value == null || value === '') return fallback;
        const n = Number(value);
        return Number.isFinite(n) ? n : fallback;
    }

    function snapToGridValue(value, step) {
        if(!Number.isFinite(value) || !Number.isFinite(step) || step <= 0) return value;
        return Math.round((Math.round(value / step) * step) * 1000000) / 1000000;
    }

    function snapEdgeToGridValue(value, step, mode = 'nearest') {
        if(!Number.isFinite(value) || !Number.isFinite(step) || step <= 0) return value;
        const ratio = value / step;
        let snappedIndex;
        if(mode === 'expand') snappedIndex = Math.ceil(ratio - 1e-9);
        else if(mode === 'shrink') snappedIndex = Math.floor(ratio + 1e-9);
        else snappedIndex = Math.round(ratio);
        return Math.round((snappedIndex * step) * 1000000) / 1000000;
    }

    function getSnapDirection(currentValue, previousValue) {
        if(!Number.isFinite(currentValue) || !Number.isFinite(previousValue)) return 'nearest';
        if(currentValue > previousValue + 1e-9) return 'expand';
        if(currentValue < previousValue - 1e-9) return 'shrink';
        return 'nearest';
    }

    function snapRectToGrid(rect, snapCfg, previousRect = null) {
        if(!snapCfg || !snapCfg.enabled) return { ...rect };
        const prev = previousRect || rect;
        const snappedX = snapEdgeToGridValue(rect.x, snapCfg.snapX, getSnapDirection(rect.x, prev.x));
        const snappedY = snapEdgeToGridValue(rect.y, snapCfg.snapY, getSnapDirection(rect.y, prev.y));
        const snappedRight = snapEdgeToGridValue(rect.x + rect.w, snapCfg.snapX, getSnapDirection(rect.x + rect.w, prev.x + prev.w));
        const snappedBottom = snapEdgeToGridValue(rect.y + rect.h, snapCfg.snapY, getSnapDirection(rect.y + rect.h, prev.y + prev.h));
        return {
            x: snappedX,
            y: snappedY,
            w: Math.max(snapCfg.snapX, Math.round((snappedRight - snappedX) * 1000000) / 1000000),
            h: Math.max(snapCfg.snapY, Math.round((snappedBottom - snappedY) * 1000000) / 1000000)
        };
    }

    function getGeomAxisStep(axis, snapCfg = null) {
        const cfg = snapCfg || getGridSnapConfig();
        if(cfg && cfg.enabled) {
            if(axis === 'x' || axis === 'w') return cfg.snapX;
            if(axis === 'y' || axis === 'h') return cfg.snapY;
        }
        return 0.01;
    }

    function formatStepAttr(step, fallback = '0.01') {
        if(!Number.isFinite(step) || step <= 0) return fallback;
        return step.toFixed(6).replace(/0+$/, '').replace(/\.$/, '') || fallback;
    }

    function getGeomDisplayDecimals(step) {
        const formatted = formatStepAttr(step, '0.001');
        const dotIndex = formatted.indexOf('.');
        return dotIndex >= 0 ? Math.max(3, formatted.length - dotIndex - 1) : 3;
    }

    function formatGeomValue(value, axis, snapCfg = null) {
        const step = getGeomAxisStep(axis, snapCfg);
        const decimals = getGeomDisplayDecimals(step);
        const safeValue = Number.isFinite(Number(value)) ? Number(value) : 0;
        return safeValue.toFixed(decimals);
    }

    function refreshGeomInputs(rect = null) {
        const snapCfg = getGridSnapConfig();
        const xStep = formatStepAttr(getGeomAxisStep('x', snapCfg));
        const yStep = formatStepAttr(getGeomAxisStep('y', snapCfg));
        const wStep = formatStepAttr(getGeomAxisStep('w', snapCfg));
        const hStep = formatStepAttr(getGeomAxisStep('h', snapCfg));
        const xInput = document.getElementById('p_x');
        const yInput = document.getElementById('p_y');
        const wInput = document.getElementById('p_w');
        const hInput = document.getElementById('p_h');
        if(xInput) xInput.step = xStep;
        if(yInput) yInput.step = yStep;
        if(wInput) wInput.step = wStep;
        if(hInput) hInput.step = hStep;
        if(rect) {
            if(xInput) xInput.value = formatGeomValue(rect.x, 'x', snapCfg);
            if(yInput) yInput.value = formatGeomValue(rect.y, 'y', snapCfg);
            if(wInput) wInput.value = formatGeomValue(rect.w, 'w', snapCfg);
            if(hInput) hInput.value = formatGeomValue(rect.h, 'h', snapCfg);
        }
    }

    function updateGridVis() {
        if(!workArea?.isConnected) return;
        const cfg = getGridSnapConfig();
        if(!cfg.enabled) {
            workArea.style.backgroundSize = '';
            refreshGeomInputs(selectedObj());
            return;
        }
        workArea.style.backgroundSize = `${cfg.cellPxX}px ${cfg.cellPxY}px`;
        refreshGeomInputs(selectedObj());
    }

    function updateGridSnapX() {
        markHistoryDirty();
        const xInput = document.getElementById('grid_snap_x');
        let val = parseFloat(xInput.value);
        if(!Number.isFinite(val) || val < 0) val = 0;
        xInput.value = val;
        syncGridSnapYWithAspect();
        updateGridVis();
        renderAll();
    }

    function updateGridSnapY() {
        markHistoryDirty();
        const yInput = document.getElementById('grid_snap_y');
        let val = parseFloat(yInput.value);
        if(!Number.isFinite(val) || val < 0) val = 0;
        yInput.value = val;
        updateGridVis();
        renderAll();
    }

    function toggleGridSnapAutoY() {
        const yInput = document.getElementById('grid_snap_y');
        const autoY = document.getElementById('grid_snap_auto_y');
        if(yInput) yInput.disabled = !!(autoY && autoY.checked);
        syncGridSnapYWithAspect();
        updateGridVis();
        renderAll();
    }

    function readFileAsDataURL(file) {
        return new Promise((resolve, reject) => {
            const reader = new FileReader();
            reader.onload = () => resolve(reader.result);
            reader.onerror = () => reject(reader.error || new Error('读取文件失败'));
            reader.readAsDataURL(file);
        });
    }

    function readFileAsText(file) {
        return new Promise((resolve, reject) => {
            const reader = new FileReader();
            reader.onload = () => resolve(String(reader.result || ''));
            reader.onerror = () => reject(reader.error || new Error('读取文本失败'));
            reader.readAsText(file);
        });
    }

    function normalizeShaderSource(text) {
        return String(text || '').replace(/\r\n/g, '\n');
    }

    function getComponentTextSource(component) {
        return normalizeShaderSource(component && component.textContent ? component.textContent : '');
    }

    function getComponentRenderedText(component, valueReplacement = '100') {
        const textSource = getComponentTextSource(component);
        return component && component.valVar ? textSource.replace(/\{val\}/g, String(valueReplacement)) : textSource;
    }

    function getComponentRenderedTextTokens(component, valueReplacement = '100') {
        const source = getComponentTextSource(component);
        const replacement = String(valueReplacement);
        const tokens = [];
        for(let i = 0; i < source.length;) {
            if(component && component.valVar && source.startsWith('{val}', i)) {
                const digits = Array.from(replacement);
                digits.forEach((char, dynamicDigitIndex) => tokens.push({
                    char,
                    sourceIndex: i,
                    isDynamic: true,
                    dynamicDigitIndex,
                    dynamicDigitCount: digits.length
                }));
                i += 5;
                continue;
            }
            const codePoint = source.codePointAt(i);
            const char = String.fromCodePoint(codePoint);
            tokens.push({ char, sourceIndex: i, isDynamic: false, dynamicDigitIndex: -1, dynamicDigitCount: 0 });
            i += char.length;
        }
        return tokens;
    }

    function getComponentTextAssetSource(component) {
        const textSource = getComponentTextSource(component);
        return component && component.valVar ? textSource.replace(/\{val\}/g, '') : textSource;
    }

    function renderAutoFunctionPreview(component, previewError = '') {
        const wrap = document.getElementById('auto_func_preview_wrap');
        const canvas = document.getElementById('auto_func_preview');
        const caption = document.getElementById('auto_func_preview_caption');
        if(!wrap || !canvas || !caption) return;
        if(!component || getAutoSourceMode(component) !== 'function') {
            wrap.style.display = 'none';
            return;
        }

        const ctx = canvas.getContext('2d');
        const w = canvas.width;
        const h = canvas.height;
        ctx.clearRect(0, 0, w, h);

        ctx.fillStyle = 'rgba(8, 12, 18, 1)';
        ctx.fillRect(0, 0, w, h);

        ctx.strokeStyle = 'rgba(255,255,255,0.08)';
        ctx.lineWidth = 1;
        for(let gx = 0; gx <= 4; gx++) {
            const x = (gx / 4) * w;
            ctx.beginPath();
            ctx.moveTo(x, 0);
            ctx.lineTo(x, h);
            ctx.stroke();
        }
        for(let gy = 0; gy <= 4; gy++) {
            const y = (gy / 4) * h;
            ctx.beginPath();
            ctx.moveTo(0, y);
            ctx.lineTo(w, y);
            ctx.stroke();
        }

        let samplesX = null;
        let samplesY = null;
        let errorText = previewError;
        if(!errorText) {
            try {
                samplesX = buildAutoFunctionSamples(component, 'x');
                samplesY = component.type === 'joystick' ? buildAutoFunctionSamples(component, 'y') : null;
            } catch(err) {
                errorText = err && err.message ? err.message : '函数预览失败';
            }
        }

        if(errorText) {
            ctx.strokeStyle = 'rgba(255,95,116,0.4)';
            ctx.lineWidth = 2;
            ctx.beginPath();
            ctx.moveTo(12, 12);
            ctx.lineTo(w - 12, h - 12);
            ctx.moveTo(w - 12, 12);
            ctx.lineTo(12, h - 12);
            ctx.stroke();
            caption.style.color = '#ff8a96';
            caption.innerText = `预览错误：${errorText}`;
            wrap.style.display = 'block';
            return;
        }

        const drawSeries = (samples, color, minVal, maxVal) => {
            if(!samples || samples.length === 0) return;
            const span = (maxVal - minVal) || 1;
            ctx.strokeStyle = color;
            ctx.lineWidth = 2;
            ctx.beginPath();
            samples.forEach((value, idx) => {
                const x = (idx / (samples.length - 1 || 1)) * (w - 1);
                const norm = (value - minVal) / span;
                const y = h - norm * h;
                if(idx === 0) ctx.moveTo(x, y);
                else ctx.lineTo(x, y);
            });
            ctx.stroke();
        };

        if(component.type === 'joystick') {
            drawSeries(samplesX, '#3fd9ff', -1, 1);
            drawSeries(samplesY, '#ffb54d', -1, 1);
            ctx.strokeStyle = 'rgba(255,255,255,0.18)';
            ctx.beginPath();
            ctx.moveTo(0, h * 0.5);
            ctx.lineTo(w, h * 0.5);
            ctx.stroke();
            caption.style.color = '#888';
            caption.innerText = `X = X(t) | Y = Y(t) | t=0..1 | 采样帧 ${Math.max(1, Math.round(component.chaosRate || 96))}`;
        } else {
            drawSeries(samplesX, '#3fd9ff', 0, 1);
            caption.style.color = '#888';
            caption.innerText = `f = f(t) | t=0..1 | 采样帧 ${Math.max(1, Math.round(component.chaosRate || 96))}`;
        }

        wrap.style.display = 'block';
    }

    function ensureEmbeddedAssetState(component) {
        if(!component.paths) component.paths = {bg: ''};
        if(!component.preview) component.preview = {};
        if(!component.embeddedAssets) component.embeddedAssets = {};
        if(component.type === 'sequence' && !component.frames) component.frames = [];

        Object.entries(component.embeddedAssets).forEach(([key, asset]) => {
            if(asset && asset.dataUrl) component.preview[key] = asset.dataUrl;
        });

        if(component.type === 'sequence') {
            component.frames.forEach(frame => {
                if(frame && frame.dataUrl) frame.preview = frame.dataUrl;
                else if(!frame || typeof frame.preview !== 'string' || !frame.preview.startsWith('data:')) frame.preview = null;
            });
        }
    }

    function collectMissingEmbeddedAssets(componentList) {
        const defaultAssets = new Set([
            '0.png', '1.png', '2.png', '3.png',
            DEFAULT_ASSET_PATHS.staticImg,
            DEFAULT_ASSET_PATHS.toggleOff,
            DEFAULT_ASSET_PATHS.toggleOn,
            DEFAULT_ASSET_PATHS.toggleProgressOff,
            DEFAULT_ASSET_PATHS.toggleProgressOn,
            DEFAULT_ASSET_PATHS.fxWhite,
            'draw_2d.hlsl',
            'draw_2d_fx.hlsl'
        ]);
        const missingAssets = new Set();

        componentList.forEach(component => {
            const paths = component.paths || {};
            const embeddedAssets = component.embeddedAssets || {};

            Object.entries(paths).forEach(([key, relPath]) => {
                if(!relPath) return;
                const assetName = relPath.split(/[\\/]/).pop();
                if(embeddedAssets[key] && embeddedAssets[key].dataUrl) return;
                if(defaultAssets.has(assetName)) return;
                missingAssets.add(relPath);
            });

            if(component.type === 'sequence') {
                (component.frames || []).forEach(frame => {
                    if(!frame || !frame.path || frame.dataUrl) return;
                    missingAssets.add(frame.path);
                });
            }
        });

        return Array.from(missingAssets).sort();
    }

    function normalizeAssetPath(assetPath) {
        return String(assetPath || '').replace(/\s+/g, '_');
    }

    function normalizeZipAssetPath(assetPath) {
        const raw = normalizeAssetPath(assetPath).trim();
        if(!raw) return '';
        return raw.replace(/\\/g, '/').replace(/^\.\//, '');
    }

    function resolveAssetFetchUrl(assetPath) {
        const raw = String(assetPath || '').trim();
        if(!raw) return '';
        if(/^data:/i.test(raw)) return raw;
        if(/^[A-Za-z][A-Za-z0-9+.-]*:/i.test(raw)) return raw;
        if(/^[A-Za-z]:[\\/]/.test(raw)) {
            return encodeURI(`file:///${raw.replace(/\\/g, '/')}`);
        }
        if(/^\\\\/.test(raw)) {
            return encodeURI(`file:${raw.replace(/\\/g, '/')}`);
        }
        return new URL(raw, window.location.href).href;
    }

    function collectAssetPackageEntries(componentList, proceduralAssets) {
        const entries = new Map();
        const defaultProcedural = proceduralAssets || {};

        componentList.forEach((component) => {
            if(!component) return;
            normalizeComponentState(component);
            const paths = component.paths || {};
            const embeddedAssets = component.embeddedAssets || {};

            Object.entries(paths).forEach(([key, relPath]) => {
                if(!relPath) return;
                const zipPath = normalizeZipAssetPath(relPath);
                if(!zipPath) return;
                if(embeddedAssets[key] && embeddedAssets[key].dataUrl) {
                    entries.set(zipPath, { type: 'dataUrl', source: embeddedAssets[key].dataUrl, origin: relPath });
                } else if(defaultProcedural[relPath]) {
                    entries.set(zipPath, { type: 'dataUrl', source: defaultProcedural[relPath], origin: relPath });
                } else {
                    entries.set(zipPath, { type: 'external', source: relPath, origin: relPath });
                }
            });

            if(component.type === 'sequence') {
                (component.frames || []).forEach((frame) => {
                    if(!frame || !frame.path) return;
                    const zipPath = normalizeZipAssetPath(frame.path);
                    if(!zipPath) return;
                    if(frame.dataUrl) {
                        entries.set(zipPath, { type: 'dataUrl', source: frame.dataUrl, origin: frame.path });
                    } else if(defaultProcedural[frame.path]) {
                        entries.set(zipPath, { type: 'dataUrl', source: defaultProcedural[frame.path], origin: frame.path });
                    } else {
                        entries.set(zipPath, { type: 'external', source: frame.path, origin: frame.path });
                    }
                });
            }
        });

        return entries;
    }

    function buildFontAssetDataMap() {
        const size = parseInt(document.getElementById('gen_tex_size').value) || 128;
        const renderTasks = new Map();

        components.forEach(m => {
            if(m.type !== 'text' || !m.textContent) return;

            const defaultColor = m.fontColor || '#ffffff';
            const overrides = m.colorOverrides || {};
            const fontStr = getTextFontCss(m, size * 0.75);
            const charsToProcess = [];

            getComponentRenderedTextTokens(m, '000').forEach(token => {
                if(token.char === '\n' || !token.char.trim()) return;
                charsToProcess.push({
                    char: token.char,
                    color: token.isDynamic ? defaultColor : (overrides[token.sourceIndex] || defaultColor),
                    fontStr
                });
            });

            if(m.valVar) {
                for(let i = 0; i <= 9; i++) charsToProcess.push({char: i.toString(), color: defaultColor, fontStr});
            }

            charsToProcess.forEach(item => {
                const resourceKey = getTextGlyphResourceKey(m, item.char, item.color);
                if(!renderTasks.has(resourceKey)) {
                    renderTasks.set(resourceKey, {
                        fileName: `${resourceKey}.png`,
                        char: item.char,
                        color: item.color,
                        fontStr: item.fontStr,
                        widthRatio: getTextGlyphAdvanceRatio(m, item.char)
                    });
                }
            });
        });

        if(renderTasks.size === 0) return {};

        const canvas = document.createElement('canvas');
        const fontAssets = {};

        for(const task of renderTasks.values()) {
            canvas.width = Math.max(2, Math.round(size * task.widthRatio));
            canvas.height = size;
            const ctx = canvas.getContext('2d');
            ctx.clearRect(0, 0, canvas.width, canvas.height);
            // 注意：不要再做 Y 轴翻转。游戏端 draw_2d.hlsl 以 UV(0,0)=左上角
            // 直接 tex.Load 采样，PNG 顶行对应屏幕顶部，正立的位图才会正立显示。
            ctx.font = task.fontStr;
            ctx.fillStyle = task.color;
            ctx.textAlign = 'center';
            ctx.textBaseline = 'middle';
            ctx.fillText(task.char, canvas.width / 2, size / 2);
            fontAssets[task.fileName] = canvas.toDataURL('image/png');
        }

        return fontAssets;
    }

    async function savePreset() {
        if(!validateShortcutSettings()) {
            alert('请先修正快捷键设置。');
            return;
        }
        const data = buildV64PresetData();
        data.generated.missingAssets = collectMissingEmbeddedAssets(data.components);

        const json = JSON.stringify(data, null, 2);
        const fallbackDownload = () => {
            const blob = new Blob([json], {type: "application/json"});
            const url = URL.createObjectURL(blob);
            const a = document.createElement('a');
            a.href = url;
            a.download = 'ui_standalone_' + data.hash + '_' + Date.now() + '.json';
            document.body.appendChild(a);
            a.click();
            document.body.removeChild(a);
            URL.revokeObjectURL(url);
        };

        try {
            if (ssmtHostBridge.available()) {
                const result = await ssmtHostBridge.request('save-preset', { json: json, hash: data.hash || '' });
                alert('预设已保存到当前工作空间：\n' + result.path);
            } else {
                fallbackDownload();
            }
        } catch (e) {
            if (e && e.message === 'NO_HOST') {
                fallbackDownload();
            } else {
                alert('保存预设失败：' + (e && e.message ? e.message : e));
            }
        }

        if(data.generated.missingAssets.length > 0) {
            alert("以下资源尚未嵌入当前预设，保存后仍需要手动放入：\n\n" + data.generated.missingAssets.join("\n"));
        }
    }

    function applyPresetData(data) {
        if(!data || !Array.isArray(data.components)) throw new Error('当前文件不是受支持的预设格式。');
        const migrationNotes = ['64', '65', '66'].includes(String(data.version || ''))
            ? loadV64Preset(data)
            : migrateLegacyPreset(data);
        previewAnimRuntime.clear();
        clearPreviewSimulationCaches();
        document.getElementById('char_hash').value = data.hash || 'c209c22b';
        document.getElementById('match_index').value = data.matchIndex || '';
        document.getElementById('match_first_index').value = data.matchFirstIndex || '';
        document.getElementById('global_aspect').value = data.aspect || 1.777;
        document.getElementById('grid_snap_x').value = data.gridSnapX || data.grid || 0.02;
        document.getElementById('grid_snap_auto_y').checked = data.gridSnapAutoY !== false;
        applyShortcutSettings(data.shortcuts || {});
        if(data.gridSnapY) document.getElementById('grid_snap_y').value = data.gridSnapY;
        else syncGridSnapYWithAspect();
        const loadedPersistentSpeed = data.persistentAnim && data.persistentAnim.speedMultiplier != null
            ? Number(data.persistentAnim.speedMultiplier)
            : PERSISTENT_ANIM_DEFAULTS.speedMultiplier;
        document.getElementById('p_anim_persistent_enabled').checked = data.persistentAnim
            ? data.persistentAnim.enabled !== false
            : PERSISTENT_ANIM_DEFAULTS.enabled;
        document.getElementById('p_anim_persistent_speed').value = Number.isFinite(loadedPersistentSpeed) && loadedPersistentSpeed >= 0 && loadedPersistentSpeed <= 0.5
            ? loadedPersistentSpeed
            : PERSISTENT_ANIM_DEFAULTS.speedMultiplier;
        document.getElementById('p_anim_persistent_flow_speed').value = data.persistentAnim && data.persistentAnim.flowSpeedScale != null
            ? data.persistentAnim.flowSpeedScale
            : PERSISTENT_ANIM_DEFAULTS.flowSpeedScale;
        toggleGridSnapAutoY();
        getPersistentAnimSettings();
        updateAspectRatio();
        renderHierarchyPanel();
        if(!applyPresetEditorLayout(data.editorLayout)) {
            clearSelection();
            propPanel.style.display = 'none';
            if(componentPanelWrap) componentPanelWrap.style.display = 'none';
            refreshAnimationPanelVisibility(null);
            renderAll();
        }
        return migrationNotes;
    }

    function loadPreset(input) {
        if(!input.files || !input.files[0]) return;
        markHistoryDirty();
        const reader = new FileReader();
        reader.onload = (e) => {
            try {
                const data = JSON.parse(e.target.result);
                const migrationNotes = applyPresetData(data);
                alert(migrationNotes && migrationNotes.length > 0
                    ? `预设导入成功。\n\n迁移结果：\n- ${migrationNotes.join('\n- ')}`
                    : "预设导入成功。");
            } catch(err) {
                console.error(err);
                const msg = err.message || String(err);
                alert("预设文件无效或已损坏：" + msg);
            }
            input.value = '';
        };
        reader.readAsText(input.files[0]);
    }

    function groupSelected() {
        markHistoryDirty();
        const selected = dedupeEntityRefs(selectedEntities);
        if(selected.length === 0) return alert("至少选择一个组件或编组后才能编组。");
        const parentLookup = buildParentLookup();
        const parentIds = [...new Set(selected.map(ref => getParentGroupIdForEntity(ref, parentLookup) || ROOT_ENTITY_ID))];
        if(parentIds.length > 1) return alert("只能对同一父级下的项目进行编组。");
        const parentKey = parentIds[0] || ROOT_ENTITY_ID;
        const parentGroupId = parentKey === ROOT_ENTITY_ID ? null : parentKey;
        const siblingInfo = getSiblingCollectionAndIndex(selected[0]);
        const siblingList = siblingInfo.list || [];
        const siblingKeys = new Set(selected.map(entityRefKey));
        const orderedSelection = siblingList.filter(ref => siblingKeys.has(entityRefKey(ref)));
        if(orderedSelection.length !== selected.length) return alert("编组失败：存在跨层级选择。");

        const group = createGroupRecord();
        group.children = orderedSelection.map(ref => ({ type: ref.type, id: ref.id }));
        const anchorComponent = orderedSelection
            .flatMap(ref => getDescendantComponents(ref))
            .sort((a, b) => ((Number(b.zIndex) || 0) - (Number(a.zIndex) || 0)))[0] || selectedObj();
        if(anchorComponent) group.globalAnim = cloneDeep(getEffectiveGlobalAnim(anchorComponent));
        removeChildFromParent(makeEntityRef('group', group.id));
        orderedSelection.forEach(removeChildFromParent);
        groups.push(normalizeGroupRecord(group));
        const insertIndex = siblingInfo.index >= 0 ? siblingInfo.index : siblingList.length;
        insertChildrenAtParent(parentGroupId, insertIndex, [makeEntityRef('group', group.id)]);
        ensureHierarchyIntegrity();
        selectEntity(makeEntityRef('group', group.id));
        renderHierarchyPanel();
        renderAll();
    }
    function ungroupSelected() {
        markHistoryDirty();
        const group = selectedGroup();
        if(!group) return alert("请先选中一个编组。");
        const ref = makeEntityRef('group', group.id);
        const siblingInfo = getSiblingCollectionAndIndex(ref);
        const insertIndex = siblingInfo.index >= 0 ? siblingInfo.index : siblingInfo.list.length;
        removeChildFromParent(ref);
        groups = groups.filter(item => item.id !== group.id);
        insertChildrenAtParent(siblingInfo.parentGroupId, insertIndex, group.children || []);
        ensureHierarchyIntegrity();
        clearSelection();
        renderHierarchyPanel();
        renderAll();
    }

    function addComponent(type) {
        markHistoryDirty();
        const id = 'comp_' + nextUniqueToken();
        let mod = {
            id, type, x: 0.35, y: 0.35, w: 0.25, h: 0.15, zIndex: 10, rot: 0,
            switchGroup: 0, initialValue: 0,
            handleSize: DEFAULT_HS, trackThick: DEFAULT_TT, cornerRadius: null,
            physics: (type === 'joystick'), paramMode: (type === 'joystick' ? '4' : '1'), 
            vars: [], paths: {bg: ''}, preview: {}, embeddedAssets: {}, 
            textContent: "Text", charSize: 0.03, lineGap: 0.0, valVar: "", textFlow: "horizontal",
            fontFamily: "Microsoft YaHei", fontColor: "#ffffff", fontBold: true, fontItalic: false,
            colorOverrides: {}, textVisibilityEnabled: false, visVar: '', visDefault: true,
            triggerCooldownSeconds: 0, lifetimeSeconds: 0,
            textHoverEffect: false, textClickVar: '',
            seqVar: "$State", frames: [],
            springK: 0.05, springD: 0.95, physicsProfile: 'normal',
            maxVals: [], defVals: [], depTargets: [], gridDepTargets: [],
            autoAnimate: false, autoStr: 0.1, autoSource: 'chaos', autoAmpX: 1, autoAmpY: 1, autoSeedX: 0.3187, autoSeedY: 0.6123, autoFuncX: (type === 'joystick' ? 'sin(TAU * t)' : 'sin01(t)'), autoFuncY: 'cos(TAU * t)', autoSpeed: 0.015, autoResponse: 0.22, autoBounce: 0.25, gravity: 0, chaosRate: 96,
            linkedSlaves: [],
            sliderSubdivisions: 1,
            zoneDragEnabled: false, zoneDragVar: DEFAULT_ZONE_DRAG_VAR, zoneDragZoneId: 0,
            joystickDirectionCount: 4, joystickSubdivisions: 1, joystickAngleOffset: 0,
            joystickDefaultX: 0, joystickDefaultY: 0, toggleSteps: DEFAULT_TOGGLE_STEPS,
            globalAnim: JSON.parse(JSON.stringify(GLOBAL_ANIM_DEFAULTS)),
            localAnim: { mode: getDefaultLocalAnimMode(type), strength: LOCAL_ANIM_DEFAULTS.strength, speed: LOCAL_ANIM_DEFAULTS.speed }
        };

        let ts = nextUniqueToken();
        if(type==='slider_h') { mod.w=0.4; mod.h=0.15; mod.minVals=[0]; mod.maxVals=[1]; mod.vars=[`$Param_${ts}`]; mod.defVals=[0]; mod.depTargets=[]; Object.assign(mod.paths, getDefaultComponentPaths(type)); }
        else if(type==='slider_v') { mod.w=0.15; mod.h=0.4; mod.minVals=[0]; mod.maxVals=[1]; mod.vars=[`$Param_${ts}`]; mod.defVals=[0]; mod.depTargets=[]; Object.assign(mod.paths, getDefaultComponentPaths(type)); }
        else if(type==='joystick') { mod.w=0.3; mod.h=0.3 * (1.777); mod.vars=[]; mod.maxVals=[]; mod.defVals=[]; mod.depTargets=[]; Object.assign(mod.paths, getDefaultComponentPaths(type)); }
            else if(type==='toggle') { mod.w=0.15; mod.h=0.2; mod.vars=[`$Toggle_${ts}`]; Object.assign(mod.paths, getDefaultComponentPaths(type)); mod.physics=false; }
        else if(type==='accum') { mod.w=0.4; mod.h=0.15; mod.vars=[]; mod.physics=false; mod.accumBindings=[]; mod.accumThreshold=5; mod.accumDirection='h'; mod.accumTriggers=[]; Object.assign(mod.paths, getDefaultComponentPaths(type)); }
        else if(type==='static') { mod.w=0.3; mod.h=0.3; mod.vars=[]; Object.assign(mod.paths, getDefaultComponentPaths(type)); mod.physics=false; mod.zIndex=5; }
        else if(type==='text') { mod.w=0.4; mod.h=0.2; mod.vars=[]; mod.physics=false; mod.zIndex=20; mod.textContent = 'Text {val}'; mod.visVar = getDefaultTextVisibilityVar(mod); }
        else if(type==='sequence') { mod.w=0.3; mod.h=0.3; mod.vars=[]; mod.physics=false; mod.zIndex=15; mod.frames=[{val:0, path:'', preview:null, dataUrl:null}]; }

        const spawnPos = findComponentSpawnPosition(mod.w, mod.h);
        mod.x = spawnPos.x;
        mod.y = spawnPos.y;
        normalizeComponentState(mod);
        components.push(mod);
        roots.push(makeEntityRef('component', id));
        ensureHierarchyIntegrity();
        renderHierarchyPanel();
        renderAll(); 
        selectItem(id, false);
    }

    function copyPropsToSelection() {
        markHistoryDirty();
        if(selectedIds.length < 2) return;
        const source = components.find(m => m.id === selectedId);
        if(!source) return;
        let count = 0;
        selectedIds.forEach(sid => {
            if(sid === selectedId) return;
            const target = components.find(m => m.id === sid);
            if(target && target.type === source.type) {
                target.w = source.w; target.h = source.h; target.zIndex = source.zIndex; target.rot = source.rot;
                target.cornerRadius = source.cornerRadius;
                target.followCursor = source.followCursor === true;
                target.followOffsetX = source.followOffsetX;
                target.followOffsetY = source.followOffsetY;
                target.zoneDragEnabled = source.zoneDragEnabled === true;
                target.zoneDragVar = getZoneDragVarName(source);
                target.zoneDragZoneId = getZoneDragZoneId(source);
                target.paths = cloneDeep(source.paths || { bg: '' });
                target.preview = cloneDeep(source.preview || {});
                target.embeddedAssets = cloneDeep(source.embeddedAssets || {});
                target.resourceOpacity = cloneDeep(source.resourceOpacity || {});
                if(target.type === 'sequence') target.frames = cloneDeep(source.frames || []);
                if(target.type.includes('slider')) { target.handleSize = source.handleSize; target.trackThick = source.trackThick; }
                if(target.type === 'toggle') {
                         target.switchGroup = source.switchGroup;
                         target.vars = [...source.vars];
                         target.paramMode = source.paramMode;
                         target.toggleSteps = source.toggleSteps;
                         target.initialValue = source.initialValue;
                         target.toggleInvert = source.toggleInvert === true;
                         normalizeToggleState(target);
                }
                if(target.type === 'accum') {
                         target.accumBindings = cloneDeep(source.accumBindings || []);
                         target.accumThreshold = source.accumThreshold;
                         target.accumDirection = source.accumDirection;
                         target.accumTriggers = cloneDeep(source.accumTriggers || []);
                }
                if(target.type === 'text') {
                         target.charSize = source.charSize; target.lineGap = source.lineGap; 
                         target.valVar = source.valVar; 
                         target.fontFamily = source.fontFamily; target.fontColor = source.fontColor;
                         target.fontBold = source.fontBold; target.fontItalic = source.fontItalic;
                         target.textFlow = source.textFlow;
                         target.textVisibilityEnabled = source.textVisibilityEnabled === true;
                         target.visVar = getDefaultTextVisibilityVar(target);
                         target.visDefault = source.visDefault !== false;
                         target.textHoverEffect = source.textHoverEffect === true;
                         target.textClickVar = getTextClickVar(source);
                         target.triggerCooldownSeconds = Math.max(0, Number(source.triggerCooldownSeconds) || 0);
                         target.lifetimeSeconds = Math.max(0, Number(source.lifetimeSeconds) || 0);
                }
                if(target.type === 'sequence') target.seqVar = source.seqVar;
                target.localAnim = cloneDeep(source.localAnim || LOCAL_ANIM_DEFAULTS);
                normalizeComponentState(target);
                count++;
            }
        });
        renderAll();
        alert(`已将属性应用到 ${count} 个组件`);
    }

    function selectItem(id, isMulti = false, skipRender = false) {
        selectEntity(makeEntityRef('component', id), isMulti, skipRender);
    }

    function highlightDOM() {
        document.querySelectorAll('.node, .group-node').forEach(el => el.classList.remove('selected', 'selected-multi'));
        selectedEntities.forEach(ref => {
            const el = document.getElementById(ref.id);
            if(el) {
                const isPrimary = selectedEntity && isSameEntityRef(selectedEntity, ref);
                el.classList.add(selectedEntities.length > 1 && !isPrimary ? 'selected-multi' : 'selected');
            }
        });
    }

    function getGlobalAnimOption(mode) {
        return GLOBAL_ANIM_MODE_OPTIONS.find(item => item.value === mode) || GLOBAL_ANIM_MODE_OPTIONS[0];
    }

    function getLocalAnimOption(component, mode) {
        return getLocalAnimOptions(component ? component.type : null).find(item => item.value === mode) || getLocalAnimOptions(component ? component.type : null)[0];
    }

    function populateLocalAnimModeSelect(component) {
        const select = document.getElementById('p_anim_local_mode');
        if(!select) return;
        const options = getLocalAnimOptions(component ? component.type : null);
        select.innerHTML = options.map(item => `<option value="${item.value}">${item.label}</option>`).join('');
    }

    function getAnimationEditorTarget() {
        const group = selectedGroup();
        if(group) return { type: 'group', entity: group };
        const component = selectedObj();
        if(component) {
            const owningGroup = getOwningGroupForComponent(component);
            if(owningGroup) return { type: 'group', entity: owningGroup, component };
            return { type: 'component', entity: component };
        }
        return null;
    }

    function applyAnimationEditorState(component = selectedObj()) {
        const target = getAnimationEditorTarget();
        if(!target) return;
        const animHolder = target.entity;
        normalizeAnimationState(animHolder);
        const globalAnim = animHolder.globalAnim || GLOBAL_ANIM_DEFAULTS;
        const localAnim = component && target.type === 'component' ? (component.localAnim || LOCAL_ANIM_DEFAULTS) : LOCAL_ANIM_DEFAULTS;
        document.getElementById('p_anim_global_mode').value = globalAnim.mode || 'none';
        document.getElementById('p_anim_global_edge').value = globalAnim.edge || 'right';
        document.getElementById('p_anim_global_strength').value = globalAnim.strength ?? GLOBAL_ANIM_DEFAULTS.strength;
        document.getElementById('p_anim_global_speed').value = globalAnim.speed ?? GLOBAL_ANIM_DEFAULTS.speed;
        document.getElementById('p_anim_global_reveal').value = globalAnim.reveal ?? GLOBAL_ANIM_DEFAULTS.reveal;
        document.getElementById('p_anim_global_trigger').value = globalAnim.trigger ?? GLOBAL_ANIM_DEFAULTS.trigger;
        document.getElementById('p_anim_global_ease').value = globalAnim.ease ?? GLOBAL_ANIM_DEFAULTS.ease;
        populateLocalAnimModeSelect(component);
        document.getElementById('p_anim_local_mode').value = component ? (localAnim.mode || getDefaultLocalAnimMode(component.type)) : 'none';
        document.getElementById('p_anim_local_strength').value = component ? (localAnim.strength ?? LOCAL_ANIM_DEFAULTS.strength) : LOCAL_ANIM_DEFAULTS.strength;
        document.getElementById('p_anim_local_speed').value = component ? (localAnim.speed ?? LOCAL_ANIM_DEFAULTS.speed) : LOCAL_ANIM_DEFAULTS.speed;
        refreshAnimationHints(component, target);
    }

    function refreshAnimationHints(component, targetInfo = null) {
        const target = targetInfo || getAnimationEditorTarget();
        const globalMode = document.getElementById('p_anim_global_mode').value || 'none';
        const localMode = document.getElementById('p_anim_local_mode').value || getDefaultLocalAnimMode(component ? component.type : null);
        const globalOption = getGlobalAnimOption(globalMode);
        const localOption = getLocalAnimOption(component, localMode);
        const showEdge = globalMode === 'edge_dock';
        const rowGlobalStrength = document.getElementById('row_anim_global_strength');
        const edgeRows = ['row_anim_global_edge', 'row_anim_global_reveal', 'row_anim_global_trigger', 'row_anim_global_ease'];
        edgeRows.forEach(id => {
            const el = document.getElementById(id);
            if(el) el.style.display = showEdge ? 'flex' : 'none';
        });
        if(rowGlobalStrength) rowGlobalStrength.style.display = showEdge ? 'none' : 'flex';
        const globalLbl = document.getElementById('lbl_anim_global_strength');
        if(globalLbl) globalLbl.innerText = globalMode === 'group_pulse' ? '呼吸强度:' : '幅度:';
        const localLbl = document.getElementById('lbl_anim_local_strength');
        if(localLbl) {
            if(localMode === 'sheen' || localMode === 'shimmer' || localMode === 'radial_sheen') localLbl.innerText = '高光强度:';
            else if(localMode === 'toggle_slide') localLbl.innerText = '滑动强度:';
            else localLbl.innerText = '强度:';
        }
        const globalHint = document.getElementById('anim_global_hint');
        if(globalHint) {
            const scope = !target ? '当前未选中可编辑对象' :
                target.type === 'group'
                    ? '当前全局动画编辑对象是编组，效果会作用于该编组及其后代'
                    : '当前组件未被编组，全局动画只作用于当前组件';
            globalHint.innerHTML = `<strong>${globalOption.label}</strong>：${globalOption.hint}<br>${scope}`;
        }
        const localHint = document.getElementById('anim_local_hint');
        if(localHint) {
            const typeName = ({
                slider_h: '水平滑条',
                slider_v: '垂直滑条',
                joystick: '摇杆',
                toggle: '开关',
                accum: '积蓄条',
                static: '静态图',
                sequence: '序列帧',
                text: '文本'
            }[component ? component.type : ''] || '组件');
            localHint.innerHTML = `<strong>${typeName}</strong> 的独立动画说明：${localOption.hint}`;
        }
        const localTitle = document.getElementById('anim_local_title');
        if(localTitle) {
            const typeName = ({
                slider_h: '水平滑条',
                slider_v: '垂直滑条',
                joystick: '摇杆',
                toggle: '开关',
                accum: '积蓄条',
                static: '静态图片',
                sequence: '序列动画',
                text: '文本组件'
            }[component ? component.type : ''] || '组件');
            localTitle.innerText = component ? `${typeName} 独立动画` : '独立动画';
        }
    }

    function isAnimationPanelEnabled() {
        const input = document.getElementById('show_anim_panel');
        return !!(input && input.checked);
    }

    function refreshAnimationPanelVisibility(component = selectedObj()) {
        if(!animPanel) return;
        const enabled = isAnimationPanelEnabled();
        const sidebar = document.querySelector('.sidebar');
        if(!enabled) {
            animPanel.style.display = 'none';
            if(animColumn) animColumn.style.display = 'none';
            if(sidebar) sidebar.classList.remove('with-anim');
            syncWindowDockButtons();
            resize();
            return;
        }
        animPanel.style.display = 'block';
        if(animColumn) animColumn.style.display = 'block';
        if(sidebar) sidebar.classList.add('with-anim');
        const localBlock = document.getElementById('anim_local_block');
        const localEmpty = document.getElementById('anim_local_empty');
        const hasComponent = !!component;
        if(localBlock) localBlock.style.display = 'block';
        if(localEmpty) localEmpty.style.display = hasComponent ? 'none' : 'block';
        const localControls = animPanel.querySelector('.anim-local-block details');
        if(localControls) localControls.style.display = hasComponent ? 'block' : 'none';
        syncWindowDockButtons();
        resize();
    }

    function toggleAnimationPanel() {
        refreshAnimationPanelVisibility(selectedObj());
        resize();
    }

    let toolWindowZIndex = 260;

    function getToolWindowElement(name) {
        return ({
            settings: document.getElementById('settingsWindow'),
            animation: document.getElementById('animColumn'),
            properties: document.getElementById('componentPanelWrap'),
            hierarchy: document.querySelector('.hierarchy-sidebar'),
            resources: document.getElementById('resourceWindow'),
            textLogic: document.getElementById('textLogicWindow')
        })[name] || null;
    }

    function isToolWindowVisible(name) {
        const element = getToolWindowElement(name);
        if(!element) return false;
        if(name === 'animation') return isAnimationPanelEnabled() && element.style.display !== 'none';
        if(name === 'properties') return !!(propPanel && propPanel.style.display !== 'none' && element.style.display !== 'none');
        return element.style.display !== 'none';
    }

    function syncWindowDockButtons() {
        document.querySelectorAll('.window-dock [data-window]').forEach(button => {
            const name = button.dataset.window;
            const open = isToolWindowVisible(name);
            button.classList.toggle('is-open', open);
            button.setAttribute('aria-pressed', open ? 'true' : 'false');
        });
    }

    function bringToolWindowToFront(element) {
        if(!element) return;
        element.style.zIndex = String(++toolWindowZIndex);
    }

    window.toggleToolWindow = (name) => {
        if(name === 'animation') {
            const input = document.getElementById('show_anim_panel');
            if(input) {
                input.checked = !isAnimationPanelEnabled();
                refreshAnimationPanelVisibility(selectedObj());
                if(input.checked) bringToolWindowToFront(animColumn);
                resize();
            }
            return;
        }
        const element = getToolWindowElement(name);
        if(!element) return;
        if(name === 'properties' && propPanel && propPanel.style.display === 'none') return;
        const nextVisible = !isToolWindowVisible(name);
        element.style.display = nextVisible ? (name === 'hierarchy' ? 'flex' : 'block') : 'none';
        if(nextVisible) bringToolWindowToFront(element);
        syncWindowDockButtons();
        resize();
    };

    window.closeToolWindow = (name) => {
        if(name === 'animation') {
            const input = document.getElementById('show_anim_panel');
            if(input) input.checked = false;
            refreshAnimationPanelVisibility(selectedObj());
            resize();
            return;
        }
        const element = getToolWindowElement(name);
        if(element) element.style.display = 'none';
        syncWindowDockButtons();
        resize();
    };

    window.startToolWindowDrag = (event, elementId) => {
        if(!event || event.button !== 0 || (event.target && event.target.closest('button'))) return;
        const element = document.getElementById(elementId);
        if(!element) return;
        event.preventDefault();
        bringToolWindowToFront(element);
        // 悬浮窗 absolute 定位,包含块为 .ui-builder-app:坐标相对页面根计算
        const rect = element.getBoundingClientRect();
        const hostRect = (element.offsetParent || root).getBoundingClientRect();
        const startX = event.clientX;
        const startY = event.clientY;
        const originLeft = rect.left - hostRect.left;
        const originTop = rect.top - hostRect.top;
        root.classList.add('tool-window-dragging');
        const minToolWinTop = 8;
        const move = (moveEvent) => {
            element.style.left = `${Math.max(8, originLeft + moveEvent.clientX - startX)}px`;
            element.style.top = `${Math.max(minToolWinTop, originTop + moveEvent.clientY - startY)}px`;
            resize();
        };
        const stop = () => {
            document.removeEventListener('mousemove', move);
            document.removeEventListener('mouseup', stop);
            root.classList.remove('tool-window-dragging');
            resize();
        };
        __onDocument('mousemove', move);
        __onDocument('mouseup', stop);
    };

    function refreshTextLogicWindow(component = selectedObj()) {
        const windowEl = document.getElementById('textLogicWindow');
        const dockButton = document.getElementById('textLogicDockBtn');
        const isText = !!(component && component.type === 'text');
        if(dockButton) dockButton.style.display = isText ? 'flex' : 'none';
        if(!windowEl) return;
        if(!isText) {
            windowEl.style.display = 'none';
            syncWindowDockButtons();
            return;
        }
        windowEl.style.display = 'block';
        bringToolWindowToFront(windowEl);
        document.getElementById('textLogicSelectionTitle').textContent = `文本运行参数 · ${getEntityLabel(makeEntityRef('component', component.id))}`;
        document.getElementById('tl_cooldown').value = Math.max(0, Number(component.triggerCooldownSeconds) || 0);
        document.getElementById('tl_lifetime').value = Math.max(0, Number(component.lifetimeSeconds) || 0);
        document.getElementById('tl_vis_var').value = getTextVisibilityVar(component);
        const membership = getComponentDialogueMembership(component.id);
        const select = document.getElementById('tl_step_select');
        let options = '<option value="">未加入对话步骤</option>';
        const ownerGroup = getOwningGroupForComponent(component);
        dialogueLogic.dialogues.filter(dialogue => ownerGroup && dialogue.groupId === ownerGroup.id).forEach(dialogue => {
            (dialogue.nodes || []).filter(node => node.type === 'step').forEach(node => {
                options += `<option value="${escapeHtml(dialogue.id)}|${escapeHtml(node.id)}">${escapeHtml(dialogue.name)} / ${escapeHtml(node.config && node.config.name || '步骤')}</option>`;
            });
        });
        if(ownerGroup) options += '<option value="__new">+ 在所属编组中新建步骤</option>';
        select.innerHTML = options;
        select.value = membership ? `${membership.dialogue.id}|${membership.step.id}` : '';
        const randomNode = getTextRandomNode(component.id);
        document.getElementById('tl_random_enabled').checked = !!randomNode;
        renderTextRandomEditor(component, randomNode);
        renderDialogueVariableTable();
        syncWindowDockButtons();
    }

    window.updateSelectedTextRuntimeProps = () => {
        const component = selectedObj();
        if(!component || component.type !== 'text') return;
        markHistoryDirty();
        component.triggerCooldownSeconds = Math.max(0, Number(document.getElementById('tl_cooldown').value) || 0);
        component.lifetimeSeconds = Math.max(0, Number(document.getElementById('tl_lifetime').value) || 0);
        renderAll();
    };

    window.assignSelectedTextToStep = (value) => {
        const component = selectedObj();
        if(!component || component.type !== 'text') return;
        markHistoryDirty();
        dialogueLogic.dialogues.forEach(dialogue => (dialogue.nodes || []).forEach(node => {
            if(node.type === 'step' && Array.isArray(node.config && node.config.textIds)) node.config.textIds = node.config.textIds.filter(id => id !== component.id);
        }));
        let dialogue = null;
        let step = null;
        if(value === '__new') {
            const group = getOwningGroupForComponent(component);
            if(group) {
                dialogue = getOrCreateDialogueForGroup(group.id);
                step = createBlueprintNodeRecord('step', dialogue.nodes.length);
                step.config.name = `步骤 ${dialogue.nodes.filter(node => node.type === 'step').length + 1}`;
                dialogue.nodes.push(step);
                if(!dialogue.entryNodeId) dialogue.entryNodeId = step.id;
            }
        } else if(value) {
            const parts = value.split('|');
            dialogue = dialogueLogic.dialogues.find(item => item.id === parts[0]);
            step = dialogue && dialogue.nodes.find(node => node.id === parts[1]);
        }
        if(step) {
            step.config.textIds = Array.isArray(step.config.textIds) ? step.config.textIds : [];
            if(!step.config.textIds.includes(component.id)) step.config.textIds.push(component.id);
            component.textVisibilityEnabled = true;
            component.visVar = getTextVisibilityVar(component);
            component.visDefault = false;
            previewTextVariableStates.set(component.visVar, false);
        }
        refreshTextLogicWindow(component);
        renderAll();
        if(workspaceMode === 'blueprint') renderBlueprint();
    };

    function getTextRandomNode(componentId) {
        const graphs = [dialogueLogic.main, ...dialogueLogic.dialogues];
        for(const graph of graphs) {
            const node = (graph.nodes || []).find(item => item.type === 'random' && item.config && item.config.sourceTextId === componentId);
            if(node) return { graph, node };
        }
        return null;
    }

    window.toggleSelectedTextRandom = () => {
        const component = selectedObj();
        if(!component || component.type !== 'text') return;
        markHistoryDirty();
        const existing = getTextRandomNode(component.id);
        if(document.getElementById('tl_random_enabled').checked && !existing) {
            const membership = getComponentDialogueMembership(component.id);
            const graph = membership ? membership.dialogue : dialogueLogic.main;
            const node = createBlueprintNodeRecord('random', graph.nodes.length);
            node.config.sourceTextId = component.id;
            node.config.branches = [];
            graph.nodes.push(node);
        } else if(!document.getElementById('tl_random_enabled').checked && existing) {
            existing.graph.nodes = existing.graph.nodes.filter(node => node.id !== existing.node.id);
            existing.graph.edges = (existing.graph.edges || []).filter(edge => edge.fromNodeId !== existing.node.id && edge.toNodeId !== existing.node.id);
        }
        refreshTextLogicWindow(component);
        if(workspaceMode === 'blueprint') renderBlueprint();
    };

    function renderTextRandomEditor(component, randomRef) {
        const wrap = document.getElementById('tl_random_editor');
        if(!wrap) return;
        if(!randomRef) { wrap.innerHTML = ''; return; }
        const branches = randomRef.node.config.branches || [];
        const total = branches.reduce((sum, item) => sum + Math.max(0, Math.round(Number(item.weightBp) || 0)), 0);
        const targetOptions = components.filter(item => item.type === 'text' && item.id !== component.id)
            .map(item => `<option value="${escapeHtml(item.id)}">${escapeHtml(getEntityLabel(makeEntityRef('component', item.id)))}</option>`).join('');
        wrap.innerHTML = `<div class="anim-note" style="margin:8px 0;color:${total === 10000 ? 'var(--group-color)' : '#ff8b7c'}">权重合计 ${(total / 100).toFixed(2)}%</div>` + branches.map((branch, index) => `
            <div class="input-row"><select onchange="UIB.updateTextRandomBranch(${index},'targetTextId',this.value)"><option value="">选择目标文本</option>${targetOptions}</select><input type="number" min="0" max="100" step="0.01" value="${(Number(branch.weightBp || 0) / 100).toFixed(2)}" onchange="UIB.updateTextRandomBranch(${index},'weight',this.value)"><button style="width:30px;margin:0" onclick="UIB.removeTextRandomBranch(${index})">×</button></div>`).join('') +
            `<button onclick="UIB.addTextRandomBranch()">+ 添加随机目标</button>`;
        wrap.querySelectorAll('select').forEach((select, index) => { select.value = branches[index] && branches[index].targetTextId || ''; });
    }

    window.addTextRandomBranch = () => {
        const component = selectedObj(); const ref = component && getTextRandomNode(component.id); if(!ref) return;
        markHistoryDirty(); ref.node.config.branches.push({ targetTextId: '', weightBp: 0 }); refreshTextLogicWindow(component);
    };
    window.removeTextRandomBranch = index => {
        const component = selectedObj(); const ref = component && getTextRandomNode(component.id); if(!ref) return;
        markHistoryDirty(); ref.node.config.branches.splice(index, 1); refreshTextLogicWindow(component);
    };
    window.updateTextRandomBranch = (index, key, value) => {
        const component = selectedObj(); const ref = component && getTextRandomNode(component.id); if(!ref || !ref.node.config.branches[index]) return;
        markHistoryDirty();
        if(key === 'weight') ref.node.config.branches[index].weightBp = Math.max(0, Math.min(10000, Math.round((Number(value) || 0) * 100)));
        else ref.node.config.branches[index][key] = value;
        refreshTextLogicWindow(component);
    };

    function renderDialogueVariableTable() {
        collectDialogueVariableNames();
        const wrap = document.getElementById('tl_variable_table'); if(!wrap) return;
        wrap.innerHTML = dialogueLogic.variables.length ? dialogueLogic.variables.map((item, index) => `<div class="runtime-row"><span class="var-tag">${escapeHtml(item.name)}</span><input type="number" step="any" value="${Number(item.initialValue) || 0}" onchange="UIB.updateDialogueVariableInitial(${index},this.value)"></div>`).join('') : '<div class="anim-note">蓝图尚未引用普通变量。</div>';
    }

    window.updateDialogueVariableInitial = (index, value) => {
        if(!dialogueLogic.variables[index]) return; markHistoryDirty(); dialogueLogic.variables[index].initialValue = Number(value) || 0;
    };

    function createBlueprintNodeRecord(type, index = 0) {
        const base = {
            id: `bp_${nextUniqueToken()}`, type,
            x: 380 + (index % 2) * 340, y: 120 + Math.floor(index / 2) * 300,
            config: {}
        };
        if(type === 'trigger' || type === 'condition') base.config = { mode: 'all', clauses: [{ variable: '$State', operator: '==', value: 1 }] };
        if(type === 'text') base.config = { componentId: selectedObj() && selectedObj().type === 'text' ? selectedObj().id : '' };
        if(type === 'dialogue') {
            const owner = selectedObj() && getOwningGroupForComponent(selectedObj());
            base.config = { groupId: owner ? owner.id : (groups[0] && groups[0].id || '') };
        }
        if(type === 'step') base.config = { name: `步骤 ${index + 1}`, textIds: selectedObj() && selectedObj().type === 'text' ? [selectedObj().id] : [] };
        if(type === 'action') base.config = { assignments: [{ variable: '$State', operation: 'set', value: 1 }] };
        if(type === 'random') base.config = {
            seed: Number((0.001 + ((hashStableString(base.id) % 9980) / 10000)).toFixed(4)),
            branches: [],
            branchPorts: [{ id: 'branch_1', label: '分支 1', weightBp: 5000 }, { id: 'branch_2', label: '分支 2', weightBp: 5000 }]
        };
        if(type === 'exit') base.config = { name: '结束' };
        return base;
    }

    function getBlueprintNodeLabel(node) {
        const cfg = node.config || {};
        if(node.type === 'trigger') return '条件触发';
        if(node.type === 'condition') return '条件分支';
        if(node.type === 'text') return getEntityLabel(makeEntityRef('component', cfg.componentId)) || '独立文本';
        if(node.type === 'dialogue') { const group = getGroupById(cfg.groupId); return group ? sanitizeGroupDisplayName(group.name, group.id) : '对话编组'; }
        if(node.type === 'step') return cfg.name || '对话步骤';
        if(node.type === 'action') return '变量动作';
        if(node.type === 'random') return '加权随机';
        return cfg.name || '结束/出口';
    }

    function getBlueprintNodeSummary(node) {
        const cfg = node.config || {};
        if(node.type === 'trigger' || node.type === 'condition') return `${cfg.mode === 'any' ? '任一' : '全部'}：${(cfg.clauses || []).map(c => `${c.variable} ${c.operator} ${c.value}`).join('；') || '未配置'}`;
        if(node.type === 'text') return cfg.componentId ? `显示 ${getTextVisibilityVar(getComponentById(cfg.componentId) || {})}` : '请选择文本框';
        if(node.type === 'dialogue') return cfg.groupId ? '双击进入内部蓝图' : '请选择编组';
        if(node.type === 'step') return `${(cfg.textIds || []).length} 个文本：${(cfg.textIds || []).map(id => getEntityLabel(makeEntityRef('component', id))).join('、') || '空步骤'}`;
        if(node.type === 'action') return (cfg.assignments || []).map(a => `${a.variable} ${a.operation} ${a.value ?? ''}`).join('；') || '未配置';
        if(node.type === 'random') return `种子 ${Number(cfg.seed || 0.3187).toFixed(4)} · ${(cfg.branches || []).reduce((s,b) => s + Number(b.weightBp || 0), 0) / 100}%`;
        return '结束当前对话或从命名出口返回';
    }

    window.setWorkspaceMode = mode => {
        if(mode === 'run') {
            startDialoguePreview();
            return;
        }
        stopDialoguePreview(false);
        workspaceMode = mode === 'blueprint' ? 'blueprint' : 'layout';
        document.getElementById('blueprintWorkspace').style.display = workspaceMode === 'blueprint' ? 'block' : 'none';
        workArea.style.display = workspaceMode === 'layout' ? 'block' : 'none';
        document.getElementById('runtimeInspector').style.display = 'none';
        syncWorkspaceModeButtons();
        if(workspaceMode === 'blueprint') renderBlueprint(); else renderAll();
    };

    function syncWorkspaceModeButtons() {
        document.getElementById('modeLayoutBtn').classList.toggle('active', workspaceMode === 'layout');
        document.getElementById('modeBlueprintBtn').classList.toggle('active', workspaceMode === 'blueprint');
        document.getElementById('modeRunBtn').classList.toggle('active', workspaceMode === 'run');
        document.getElementById('modeRunBtn').textContent = workspaceMode === 'run' ? '停止' : '运行';
    }

    window.openMainBlueprint = () => { blueprintScopeId = 'main'; selectedBlueprintNodeIds = []; renderBlueprint(); };
    window.openBlueprintForSelectedText = () => {
        const component = selectedObj(); if(!component || component.type !== 'text') return;
        const membership = getComponentDialogueMembership(component.id);
        blueprintScopeId = membership ? membership.dialogue.id : 'main';
        selectedBlueprintNodeIds = membership ? [membership.step.id] : [];
        setWorkspaceMode('blueprint');
    };

    window.addBlueprintNode = () => {
        const graph = getBlueprintGraph();
        const type = document.getElementById('blueprintNodeType').value;
        if(blueprintScopeId === 'main' && (type === 'step' || type === 'exit')) { alert('对话步骤和出口只能添加在对话编组内部。'); return; }
        if(blueprintScopeId !== 'main' && type === 'trigger') { alert('条件触发节点只能添加在主逻辑。'); return; }
        markHistoryDirty();
        const node = createBlueprintNodeRecord(type, graph.nodes.length);
        graph.nodes.push(node);
        if(blueprintScopeId !== 'main' && type === 'step' && !graph.entryNodeId) graph.entryNodeId = node.id;
        selectedBlueprintNodeIds = [node.id];
        renderBlueprint();
    };

    window.connectSelectedBlueprintNodes = async () => {
        if(selectedBlueprintNodeIds.length !== 2) { alert('请按顺序选择两个节点后连接。'); return; }
        const graph = getBlueprintGraph();
        const from = graph.nodes.find(node => node.id === selectedBlueprintNodeIds[0]);
        const to = graph.nodes.find(node => node.id === selectedBlueprintNodeIds[1]);
        if(!from || !to || from.id === to.id) return;
        let port = 'out';
        if(from.type === 'condition') port = (await prompt('条件出口：true 或 false', 'true')) === 'false' ? 'false' : 'true';
        if(from.type === 'step') {
            const firstText = from.config && from.config.textIds && from.config.textIds[0];
            port = (await prompt('步骤出口，例如 click:文本ID 或 timeout:文本ID', firstText ? `click:${firstText}` : 'out')) || 'out';
        }
        markHistoryDirty();
        graph.edges.push({ id: `edge_${nextUniqueToken()}`, fromNodeId: from.id, fromPort: port, toNodeId: to.id, toPort: 'in', weightBp: from.type === 'random' ? 0 : undefined });
        renderBlueprint();
    };

    window.deleteSelectedBlueprintItems = () => {
        const graph = getBlueprintGraph(); if(selectedBlueprintNodeIds.length === 0) return;
        markHistoryDirty(); const ids = new Set(selectedBlueprintNodeIds);
        graph.nodes = graph.nodes.filter(node => !ids.has(node.id));
        graph.edges = graph.edges.filter(edge => !ids.has(edge.fromNodeId) && !ids.has(edge.toNodeId));
        if(graph.entryNodeId && ids.has(graph.entryNodeId)) graph.entryNodeId = '';
        selectedBlueprintNodeIds = []; renderBlueprint();
    };

    function getBlueprintNodePorts(node, graph = getBlueprintGraph()) {
        const cfg = node.config || {};
        const inputs = node.type === 'trigger' ? [] : [{ id: 'in', label: '进入' }];
        let outputs = [];
        if(node.type === 'trigger' || node.type === 'action') outputs = [{ id: 'out', label: '继续' }];
        else if(node.type === 'condition') outputs = [{ id: 'true', label: '成立' }, { id: 'false', label: '不成立' }];
        else if(node.type === 'text') outputs = [{ id: 'click', label: '点击' }, { id: 'timeout', label: '超时' }];
        else if(node.type === 'step') {
            (cfg.textIds || []).forEach(id => {
                const label = getEntityLabel(makeEntityRef('component', id));
                outputs.push({ id: `click:${id}`, label: `点击 · ${label}` }, { id: `timeout:${id}`, label: `超时 · ${label}` });
            });
        } else if(node.type === 'random' && !cfg.sourceTextId) {
            if(!Array.isArray(cfg.branchPorts) || cfg.branchPorts.length === 0) cfg.branchPorts = [{ id: 'branch_1', label: '分支 1', weightBp: 5000 }, { id: 'branch_2', label: '分支 2', weightBp: 5000 }];
            outputs = cfg.branchPorts.map(port => ({ id: port.id, label: `${port.label} · ${(Number(port.weightBp || 0) / 100).toFixed(2)}%` }));
        }
        return { inputs, outputs };
    }

    function createBlueprintSelect(options, value, onChange, placeholder = '请选择') {
        const select = document.createElement('select');
        const empty = document.createElement('option'); empty.value = ''; empty.textContent = placeholder; select.appendChild(empty);
        options.forEach(option => { const el = document.createElement('option'); el.value = option.value; el.textContent = option.label; select.appendChild(el); });
        select.value = value || ''; select.onchange = () => { markHistoryDirty(); onChange(select.value); };
        return select;
    }

    function appendBlueprintField(wrap, labelText, control, onRemove = null) {
        const row = document.createElement('div'); row.className = 'bp-field';
        const label = document.createElement('label'); label.textContent = labelText; row.append(label, control);
        if(onRemove) { const button = document.createElement('button'); button.type = 'button'; button.className = 'bp-mini-button'; button.textContent = '×'; button.onclick = event => { event.stopPropagation(); markHistoryDirty(); onRemove(); renderBlueprint(); }; row.appendChild(button); }
        else row.appendChild(document.createElement('span'));
        wrap.appendChild(row);
    }

    function createBlueprintInput(type, value, onChange, attrs = {}) {
        const input = document.createElement('input'); input.type = type; input.value = value ?? '';
        Object.entries(attrs).forEach(([key, val]) => input.setAttribute(key, val));
        input.oninput = () => { markHistoryDirty(); onChange(type === 'number' ? Number(input.value) || 0 : input.value); collectDialogueVariableNames(); };
        return input;
    }

    function appendBlueprintAddButton(wrap, label, callback) {
        const button = document.createElement('button'); button.type = 'button'; button.className = 'bp-add-button'; button.textContent = label;
        button.onclick = event => { event.stopPropagation(); markHistoryDirty(); callback(); renderBlueprint(); };
        wrap.appendChild(button);
    }

    function createBlueprintNodeEditor(node, graph) {
        const cfg = node.config || (node.config = {}), wrap = document.createElement('div'); wrap.className = 'bp-node-editor';
        wrap.onmousedown = event => event.stopPropagation(); wrap.onclick = event => event.stopPropagation();
        const textOptions = components.filter(item => item.type === 'text').map(item => ({ value: item.id, label: getEntityLabel(makeEntityRef('component', item.id)) }));
        if(node.type === 'trigger' || node.type === 'condition') {
            appendBlueprintField(wrap, '组合', createBlueprintSelect([{value:'all',label:'全部满足 AND'},{value:'any',label:'任一满足 OR'}], cfg.mode || 'all', value => cfg.mode = value));
            cfg.clauses = Array.isArray(cfg.clauses) ? cfg.clauses : [];
            cfg.clauses.forEach((clause, index) => {
                appendBlueprintField(wrap, `条件 ${index + 1}`, createBlueprintInput('text', clause.variable || '', value => clause.variable = sanitizeIniVarToken(value, value)), () => cfg.clauses.splice(index, 1));
                appendBlueprintField(wrap, '比较', createBlueprintSelect(['==','!=','>','>=','<','<='].map(value => ({value,label:value})), clause.operator || '==', value => clause.operator = value));
                appendBlueprintField(wrap, '数值', createBlueprintInput('number', clause.value ?? 0, value => clause.value = value, { step:'any' }));
            });
            appendBlueprintAddButton(wrap, '+ 添加条件', () => cfg.clauses.push({ variable: '$State', operator: '==', value: 1 }));
        } else if(node.type === 'text') {
            appendBlueprintField(wrap, '文本框', createBlueprintSelect(textOptions, cfg.componentId, value => { cfg.componentId = value; renderBlueprint(); }, '选择文本框'));
        } else if(node.type === 'dialogue') {
            const groupOptions = groups.map(group => ({ value: group.id, label: sanitizeGroupDisplayName(group.name, group.id) }));
            appendBlueprintField(wrap, '编组', createBlueprintSelect(groupOptions, cfg.groupId, value => { cfg.groupId = value; if(value) getOrCreateDialogueForGroup(value); renderBlueprint(); }, '选择编组'));
            if(cfg.groupId) appendBlueprintAddButton(wrap, '进入编组内部蓝图', () => { blueprintScopeId = getOrCreateDialogueForGroup(cfg.groupId).id; selectedBlueprintNodeIds = []; });
        } else if(node.type === 'step') {
            appendBlueprintField(wrap, '步骤名', createBlueprintInput('text', cfg.name || '', value => cfg.name = value));
            cfg.textIds = Array.isArray(cfg.textIds) ? cfg.textIds : [];
            let allowed = textOptions;
            if(graph.groupId) allowed = getDescendantComponents(makeEntityRef('group', graph.groupId)).filter(item => item.type === 'text').map(item => ({ value:item.id, label:getEntityLabel(makeEntityRef('component', item.id)) }));
            cfg.textIds.forEach((id, index) => appendBlueprintField(wrap, `文本 ${index + 1}`, createBlueprintSelect(allowed, id, value => { cfg.textIds[index] = value; renderBlueprint(); }, '选择文本框'), () => cfg.textIds.splice(index, 1)));
            appendBlueprintAddButton(wrap, '+ 添加步骤文本', () => cfg.textIds.push(allowed[0] ? allowed[0].value : ''));
        } else if(node.type === 'action') {
            cfg.assignments = Array.isArray(cfg.assignments) ? cfg.assignments : [];
            cfg.assignments.forEach((action, index) => {
                appendBlueprintField(wrap, `变量 ${index + 1}`, createBlueprintInput('text', action.variable || '', value => action.variable = sanitizeIniVarToken(value, value)), () => cfg.assignments.splice(index, 1));
                appendBlueprintField(wrap, '操作', createBlueprintSelect([{value:'set',label:'设置'},{value:'add',label:'增加'},{value:'subtract',label:'减少'},{value:'toggle',label:'切换 0/1'}], action.operation || 'set', value => action.operation = value));
                if(action.operation !== 'toggle') appendBlueprintField(wrap, '数值', createBlueprintInput('number', action.value ?? 0, value => action.value = value, {step:'any'}));
            });
            appendBlueprintAddButton(wrap, '+ 添加变量动作', () => cfg.assignments.push({ variable:'$State', operation:'set', value:1 }));
        } else if(node.type === 'random') {
            appendBlueprintField(wrap, '固定种子', createBlueprintInput('number', cfg.seed || .3187, value => cfg.seed = Math.max(.001, Math.min(.999, value)), {min:'.001',max:'.999',step:'.0001'}));
            if(cfg.sourceTextId) {
                appendBlueprintField(wrap, '点击来源', createBlueprintSelect(textOptions, cfg.sourceTextId, value => cfg.sourceTextId = value, '选择文本框'));
                cfg.branches = Array.isArray(cfg.branches) ? cfg.branches : [];
                cfg.branches.forEach((branch, index) => {
                    appendBlueprintField(wrap, `目标 ${index + 1}`, createBlueprintSelect(textOptions, branch.targetTextId, value => branch.targetTextId = value, '选择目标'), () => cfg.branches.splice(index, 1));
                    appendBlueprintField(wrap, '百分比', createBlueprintInput('number', Number(branch.weightBp || 0) / 100, value => branch.weightBp = Math.round(Math.max(0, Math.min(100, value)) * 100), {min:'0',max:'100',step:'.01'}));
                });
                appendBlueprintAddButton(wrap, '+ 添加随机目标', () => cfg.branches.push({targetTextId:'',weightBp:0}));
            } else {
                cfg.branchPorts = Array.isArray(cfg.branchPorts) && cfg.branchPorts.length ? cfg.branchPorts : [{id:'branch_1',label:'分支 1',weightBp:5000},{id:'branch_2',label:'分支 2',weightBp:5000}];
                cfg.branchPorts.forEach((port, index) => {
                    appendBlueprintField(wrap, `分支 ${index + 1}`, createBlueprintInput('text', port.label || '', value => port.label = value), () => { cfg.branchPorts.splice(index, 1); graph.edges = graph.edges.filter(edge => !(edge.fromNodeId === node.id && edge.fromPort === port.id)); });
                    appendBlueprintField(wrap, '百分比', createBlueprintInput('number', Number(port.weightBp || 0) / 100, value => { port.weightBp = Math.round(Math.max(0, Math.min(100, value)) * 100); const edge = graph.edges.find(item => item.fromNodeId === node.id && item.fromPort === port.id); if(edge) edge.weightBp = port.weightBp; }, {min:'0',max:'100',step:'.01'}));
                });
                appendBlueprintAddButton(wrap, '+ 添加随机输出', () => { const index = cfg.branchPorts.length + 1; cfg.branchPorts.push({id:`branch_${nextUniqueToken()}`,label:`分支 ${index}`,weightBp:0}); });
            }
        } else if(node.type === 'exit') {
            appendBlueprintField(wrap, '出口名', createBlueprintInput('text', cfg.name || '结束', value => cfg.name = value));
        }
        return wrap;
    }

    function getBlueprintPortAnchor(node, direction, portId, graph = getBlueprintGraph()) {
        const ports = getBlueprintNodePorts(node, graph)[direction === 'out' ? 'outputs' : 'inputs'];
        const index = Math.max(0, ports.findIndex(port => port.id === portId));
        return { x: node.x + (direction === 'out' ? 280 : 0), y: node.y + 48 + index * 24 };
    }

    function applyBlueprintTransform(graph = getBlueprintGraph()) {
        blueprintPan = normalizeBlueprintView(graph.view);
        const transform = `translate(${blueprintPan.x}px,${blueprintPan.y}px) scale(${blueprintPan.scale})`;
        const stage = document.getElementById('blueprintStage'), svg = document.getElementById('blueprintEdges');
        if(stage) stage.style.transform = transform;
        if(svg) { svg.style.transform = transform; svg.style.transformOrigin = '0 0'; }
    }

    function renderBlueprintEdges() {
        const graph = getBlueprintGraph(), svg = document.getElementById('blueprintEdges'); if(!svg) return;
        let html = (graph.edges || []).map(edge => {
            const from = graph.nodes.find(node => node.id === edge.fromNodeId), to = graph.nodes.find(node => node.id === edge.toNodeId); if(!from || !to) return '';
            const a = getBlueprintPortAnchor(from, 'out', edge.fromPort || 'out', graph), b = getBlueprintPortAnchor(to, 'in', edge.toPort || 'in', graph), curve = Math.max(50, Math.abs(b.x - a.x) * .45);
            return `<path class="bp-edge" d="M ${a.x} ${a.y} C ${a.x + curve} ${a.y}, ${b.x - curve} ${b.y}, ${b.x} ${b.y}"/><text x="${(a.x+b.x)/2}" y="${(a.y+b.y)/2-5}" fill="currentColor" font-size="11">${escapeHtml(edge.fromPort || '')}${edge.weightBp != null ? ` ${(edge.weightBp/100).toFixed(2)}%` : ''}</text>`;
        }).join('');
        if(blueprintConnectionDraft) {
            const from = graph.nodes.find(node => node.id === blueprintConnectionDraft.fromNodeId);
            if(from) { const a = getBlueprintPortAnchor(from, 'out', blueprintConnectionDraft.fromPort, graph), b = {x:blueprintConnectionDraft.x,y:blueprintConnectionDraft.y}, curve = Math.max(50, Math.abs(b.x-a.x)*.45); html += `<path class="bp-edge-preview${blueprintConnectionDraft.mode === 'cut' ? ' cut' : ''}" d="M ${a.x} ${a.y} C ${a.x+curve} ${a.y}, ${b.x-curve} ${b.y}, ${b.x} ${b.y}"/>`; }
        }
        svg.innerHTML = html;
    }

    function renderBlueprint() {
        const graph = getBlueprintGraph(), stage = document.getElementById('blueprintStage'); if(!stage) return;
        applyBlueprintTransform(graph);
        document.getElementById('blueprintBreadcrumb').textContent = blueprintScopeId === 'main' ? '主逻辑' : `主逻辑 / ${graph.name || '对话编组'}`;
        stage.innerHTML = '';
        (graph.nodes || []).forEach(node => {
            const ports = getBlueprintNodePorts(node, graph), el = document.createElement('div');
            el.className = `bp-node${selectedBlueprintNodeIds.includes(node.id) ? ' selected' : ''}`; el.style.left = `${node.x}px`; el.style.top = `${node.y}px`; el.dataset.nodeId = node.id;
            el.style.minHeight = `${Math.max(96, 64 + Math.max(ports.inputs.length, ports.outputs.length) * 24)}px`;
            const header = document.createElement('div'); header.className = 'bp-node-title'; header.innerHTML = `<span>${escapeHtml(getBlueprintNodeLabel(node))}</span><small>${escapeHtml(node.type)}</small>`; header.onmousedown = event => startBlueprintNodeDrag(event, node, el);
            el.appendChild(header); el.appendChild(createBlueprintNodeEditor(node, graph));
            const portLayer = document.createElement('div'); portLayer.className = 'bp-node-ports';
            ports.inputs.forEach((port,index) => { const dot=document.createElement('span'); dot.className='bp-port in'; dot.style.top=`${40+index*24}px`; dot.dataset.nodeId=node.id; dot.dataset.portId=port.id; const label=document.createElement('span'); label.className='bp-port-label in'; label.style.top=`${40+index*24}px`; label.textContent=port.label; portLayer.append(dot,label); });
            ports.outputs.forEach((port,index) => { const dot=document.createElement('span'); dot.className='bp-port out'; dot.style.top=`${40+index*24}px`; dot.dataset.nodeId=node.id; dot.dataset.portId=port.id; dot.onmousedown=event=>startBlueprintConnection(event,node,port); const label=document.createElement('span'); label.className='bp-port-label out'; label.style.top=`${40+index*24}px`; label.textContent=port.label; portLayer.append(dot,label); });
            el.appendChild(portLayer);
            el.addEventListener('mousedown', event => { if(event.button === 2 && (event.altKey || event.ctrlKey)) startBlueprintQuickGesture(event, node); }, true);
            el.oncontextmenu = event => { if(event.altKey || event.ctrlKey) event.preventDefault(); };
            el.onclick = event => { if(event.target.closest('input,select,button,.bp-port')) return; event.stopPropagation(); selectedBlueprintNodeIds = event.ctrlKey ? (selectedBlueprintNodeIds.includes(node.id) ? selectedBlueprintNodeIds.filter(id=>id!==node.id) : [...selectedBlueprintNodeIds,node.id]) : [node.id]; document.querySelectorAll('.bp-node').forEach(item=>item.classList.toggle('selected',selectedBlueprintNodeIds.includes(item.dataset.nodeId))); };
            stage.appendChild(el);
        });
        renderBlueprintEdges();
    }

    function startBlueprintNodeDrag(event, node, element) {
        if(event.button !== 0) return; event.preventDefault(); event.stopPropagation(); markHistoryDirty();
        const startX=event.clientX,startY=event.clientY,x=node.x,y=node.y;
        const move=e=>{node.x=x+(e.clientX-startX)/blueprintPan.scale;node.y=y+(e.clientY-startY)/blueprintPan.scale;element.style.left=`${node.x}px`;element.style.top=`${node.y}px`;renderBlueprintEdges();};
        const stop=()=>{document.removeEventListener('mousemove',move);document.removeEventListener('mouseup',stop);}; __onDocument('mousemove',move);__onDocument('mouseup',stop);
    }

    function clientToBlueprintPoint(clientX, clientY) {
        const rect = document.getElementById('blueprintWorkspace').getBoundingClientRect();
        return { x:(clientX-rect.left-blueprintPan.x)/blueprintPan.scale, y:(clientY-rect.top-blueprintPan.y)/blueprintPan.scale };
    }

    function connectBlueprintNodes(graph, fromNode, fromPort, toNodeId, toPort = 'in') {
        if(!graph || !fromNode || !toNodeId || fromNode.id === toNodeId) return false;
        graph.edges = graph.edges.filter(edge => !(edge.fromNodeId === fromNode.id && edge.fromPort === fromPort));
        let weightBp;
        if(fromNode.type === 'random') {
            const cfgPort = (fromNode.config && fromNode.config.branchPorts || []).find(item => item.id === fromPort);
            weightBp = cfgPort ? cfgPort.weightBp : 0;
        }
        graph.edges.push({ id:`edge_${nextUniqueToken()}`, fromNodeId:fromNode.id, fromPort, toNodeId, toPort, weightBp });
        return true;
    }

    function startBlueprintConnection(event, node, port) {
        if(event.button!==0)return;event.preventDefault();event.stopPropagation();markHistoryDirty();const point=clientToBlueprintPoint(event.clientX,event.clientY);
        blueprintConnectionDraft={fromNodeId:node.id,fromPort:port.id,x:point.x,y:point.y};renderBlueprintEdges();
        const move=e=>{const p=clientToBlueprintPoint(e.clientX,e.clientY);blueprintConnectionDraft.x=p.x;blueprintConnectionDraft.y=p.y;renderBlueprintEdges();};
        const stop=e=>{const target=document.elementFromPoint(e.clientX,e.clientY);const input=target&&target.closest('.bp-port.in');const graph=getBlueprintGraph();if(input)connectBlueprintNodes(graph,node,port.id,input.dataset.nodeId,input.dataset.portId||'in');blueprintConnectionDraft=null;document.removeEventListener('mousemove',move);document.removeEventListener('mouseup',stop);renderBlueprint();};
        __onDocument('mousemove',move);__onDocument('mouseup',stop);
    }

    function startBlueprintQuickGesture(event, node) {
        event.preventDefault(); event.stopPropagation(); markHistoryDirty();
        const graph = getBlueprintGraph(), point = clientToBlueprintPoint(event.clientX, event.clientY), mode = event.ctrlKey ? 'cut' : 'connect';
        const outputs = getBlueprintNodePorts(node, graph).outputs;
        if(mode === 'connect' && outputs.length === 0) return;
        let port = outputs[0] || { id:'out' };
        if(outputs.length > 1) {
            const nearestIndex = Math.max(0, Math.min(outputs.length - 1, Math.round((point.y - node.y - 48) / 24)));
            port = outputs[nearestIndex];
        }
        blueprintConnectionDraft = { fromNodeId:node.id, fromPort:port.id, x:point.x, y:point.y, mode };
        renderBlueprintEdges();
        const move = moveEvent => { const next = clientToBlueprintPoint(moveEvent.clientX, moveEvent.clientY); blueprintConnectionDraft.x = next.x; blueprintConnectionDraft.y = next.y; renderBlueprintEdges(); };
        const stop = upEvent => {
            const target = document.elementFromPoint(upEvent.clientX, upEvent.clientY), targetNodeElement = target && target.closest('.bp-node'), targetNodeId = targetNodeElement && targetNodeElement.dataset.nodeId;
            if(targetNodeId && targetNodeId !== node.id) {
                if(mode === 'cut') graph.edges = graph.edges.filter(edge => !((edge.fromNodeId === node.id && edge.toNodeId === targetNodeId) || (edge.fromNodeId === targetNodeId && edge.toNodeId === node.id)));
                else connectBlueprintNodes(graph, node, port.id, targetNodeId, 'in');
            }
            blueprintConnectionDraft = null;
            document.removeEventListener('mousemove', move); document.removeEventListener('mouseup', stop); renderBlueprint();
        };
        __onDocument('mousemove', move); __onDocument('mouseup', stop);
    }

    function startBlueprintMarquee(event) {
        if(event.button !== 2 || event.altKey || event.ctrlKey || workspaceMode !== 'blueprint') return;
        const workspace = document.getElementById('blueprintWorkspace'), rect = workspace.getBoundingClientRect();
        const startX = event.clientX - rect.left, startY = event.clientY - rect.top;
        let moved = false;
        let marquee = workspace.querySelector('.bp-selection-marquee');
        if(!marquee) { marquee = document.createElement('div'); marquee.className = 'bp-selection-marquee'; workspace.appendChild(marquee); }
        const move = moveEvent => {
            const currentX = Math.max(0, Math.min(rect.width, moveEvent.clientX - rect.left)), currentY = Math.max(0, Math.min(rect.height, moveEvent.clientY - rect.top));
            if(!moved && Math.abs(currentX - startX) + Math.abs(currentY - startY) < 4) return;
            moved = true; moveEvent.preventDefault();
            const left = Math.min(startX, currentX), top = Math.min(startY, currentY), width = Math.abs(currentX - startX), height = Math.abs(currentY - startY);
            Object.assign(marquee.style, { display:'block', left:`${left}px`, top:`${top}px`, width:`${width}px`, height:`${height}px` });
        };
        const stop = stopEvent => {
            document.removeEventListener('mousemove', move); document.removeEventListener('mouseup', stop);
            if(moved) {
                const box = marquee.getBoundingClientRect(), selected = [];
                document.querySelectorAll('#blueprintStage .bp-node').forEach(element => {
                    const nodeRect = element.getBoundingClientRect();
                    if(nodeRect.right >= box.left && nodeRect.left <= box.right && nodeRect.bottom >= box.top && nodeRect.top <= box.bottom) selected.push(element.dataset.nodeId);
                });
                selectedBlueprintNodeIds = selected;
                document.querySelectorAll('#blueprintStage .bp-node').forEach(element => element.classList.toggle('selected', selected.includes(element.dataset.nodeId)));
                blueprintSuppressContextMenu = true;
                setTimeout(() => { blueprintSuppressContextMenu = false; }, 0);
                stopEvent.preventDefault();
            }
            marquee.style.display = 'none';
        };
        __onDocument('mousemove', move); __onDocument('mouseup', stop);
    }

    const blueprintWorkspace = document.getElementById('blueprintWorkspace');
    if(blueprintWorkspace) {
        blueprintWorkspace.addEventListener('mousedown', startBlueprintMarquee, true);
        blueprintWorkspace.onclick=event=>{if(!event.target.closest('.bp-node,.blueprint-toolbar')){selectedBlueprintNodeIds=[];document.querySelectorAll('.bp-node').forEach(item=>item.classList.remove('selected'));}};
        blueprintWorkspace.onwheel=event=>{if(workspaceMode!=='blueprint')return;event.preventDefault();markHistoryDirty();const graph=getBlueprintGraph(),rect=blueprintWorkspace.getBoundingClientRect(),oldScale=graph.view.scale||1,newScale=Math.max(.35,Math.min(2.5,oldScale*(event.deltaY<0?1.1:.9))),mx=event.clientX-rect.left,my=event.clientY-rect.top,worldX=(mx-(graph.view.x||0))/oldScale,worldY=(my-(graph.view.y||0))/oldScale;graph.view.x=mx-worldX*newScale;graph.view.y=my-worldY*newScale;graph.view.scale=newScale;applyBlueprintTransform(graph);};
        blueprintWorkspace.onmousedown=event=>{if(event.target.closest('.bp-node,.blueprint-toolbar')||(event.button!==0&&event.button!==1))return;event.preventDefault();markHistoryDirty();const graph=getBlueprintGraph(),sx=event.clientX,sy=event.clientY,ox=graph.view.x||0,oy=graph.view.y||0;blueprintWorkspace.style.cursor='grabbing';const move=e=>{graph.view.x=ox+e.clientX-sx;graph.view.y=oy+e.clientY-sy;applyBlueprintTransform(graph);};const stop=()=>{blueprintWorkspace.style.cursor='';document.removeEventListener('mousemove',move);document.removeEventListener('mouseup',stop);};__onDocument('mousemove',move);__onDocument('mouseup',stop);};
        blueprintWorkspace.oncontextmenu=event=>{if(event.altKey||event.ctrlKey||blueprintSuppressContextMenu)event.preventDefault();};
    }

    function validateDialogueLogic() {
        dialogueLogic = normalizeDialogueLogic(dialogueLogic);
        collectDialogueVariableNames();
        const errors = [];
        const validateGraph = (graph, label) => {
            const ids = new Set((graph.nodes || []).map(node => node.id));
            (graph.edges || []).forEach(edge => { if(!ids.has(edge.fromNodeId) || !ids.has(edge.toNodeId)) errors.push(`${label} 存在悬空连线`); });
            (graph.nodes || []).forEach(node => {
                const cfg = node.config || {};
                if(node.type === 'text' && (!getComponentById(cfg.componentId) || getComponentById(cfg.componentId).type !== 'text')) errors.push(`${label} 的独立文本节点未绑定有效文本框`);
                if(node.type === 'dialogue' && !getGroupById(cfg.groupId)) errors.push(`${label} 的对话编组节点未绑定有效编组`);
                if(node.type === 'step' && (!Array.isArray(cfg.textIds) || cfg.textIds.length === 0)) errors.push(`${label} 的步骤“${cfg.name || node.id}”没有文本`);
                if(node.type === 'random') {
                    const branches = Array.isArray(cfg.branches) && cfg.branches.length ? cfg.branches : (graph.edges || []).filter(edge => edge.fromNodeId === node.id);
                    const total = branches.reduce((sum, branch) => sum + Math.max(0, Math.round(Number(branch.weightBp) || 0)), 0);
                    if(total !== 10000) errors.push(`${label} 的随机节点“${getBlueprintNodeLabel(node)}”权重合计必须为 100%`);
                    if(!(Number(cfg.seed) >= .001 && Number(cfg.seed) <= .999)) errors.push(`${label} 的随机种子必须在 0.001–0.999`);
                }
            });
            const instant = new Set(['action','condition','random']);
            const visiting = new Set(), visited = new Set();
            const visit = id => {
                if(visiting.has(id)) { errors.push(`${label} 存在不经过点击或计时的即时循环`); return; }
                if(visited.has(id)) return;
                const node = (graph.nodes || []).find(item => item.id === id); if(!node || !instant.has(node.type)) return;
                visiting.add(id);
                (graph.edges || []).filter(edge => edge.fromNodeId === id).forEach(edge => visit(edge.toNodeId));
                visiting.delete(id); visited.add(id);
            };
            (graph.nodes || []).forEach(node => visit(node.id));
        };
        validateGraph(dialogueLogic.main, '主逻辑');
        const groupIds = new Set();
        dialogueLogic.dialogues.forEach(dialogue => {
            if(groupIds.has(dialogue.groupId)) errors.push(`编组 ${dialogue.groupId} 被多个对话定义引用`);
            groupIds.add(dialogue.groupId);
            if(!dialogue.entryNodeId || !(dialogue.nodes || []).some(node => node.id === dialogue.entryNodeId)) errors.push(`${dialogue.name} 没有有效入口步骤`);
            validateGraph(dialogue, dialogue.name);
        });
        return [...new Set(errors)];
    }

    function createDialogueRuntime() {
        collectDialogueVariableNames();
        const variables = new Map(dialogueLogic.variables.map(item => [item.name, Number(item.initialValue) || 0]));
        const snapshot = getPreviewSimulationSnapshot();
        if(snapshot && snapshot.variableValues) snapshot.variableValues.forEach((value, name) => variables.set(name, Number(value) || 0));
        const randomStates = new Map();
        [dialogueLogic.main, ...dialogueLogic.dialogues].forEach(graph => (graph.nodes || []).filter(node => node.type === 'random').forEach(node => randomStates.set(node.id, Number(node.config && node.config.seed) || .3187)));
        const controlledTextIds = new Set();
        (dialogueLogic.main.nodes || []).filter(node => node.type === 'text' && node.config && node.config.componentId).forEach(node => controlledTextIds.add(node.config.componentId));
        dialogueLogic.dialogues.forEach(dialogue => (dialogue.nodes || []).filter(node => node.type === 'step').forEach(node => (node.config && node.config.textIds || []).forEach(id => controlledTextIds.add(id))));
        [dialogueLogic.main, ...dialogueLogic.dialogues].forEach(graph => (graph.nodes || []).filter(node => node.type === 'random').forEach(node => {
            if(node.config && node.config.sourceTextId) controlledTextIds.add(node.config.sourceTextId);
            (node.config && node.config.branches || []).forEach(branch => { if(branch.targetTextId) controlledTextIds.add(branch.targetTextId); });
        }));
        const textStates = new Map([...controlledTextIds].map(id => {
            const component = getComponentById(id);
            return [id, { lastTrigger: -Infinity, expiresAt: 0, visible: !!(component && component.visDefault !== false) }];
        }));
        return {
            clock: 0, lastReal: performance.now() / 1000, paused: false, variables,
            textStates, dialogueStates: new Map(), triggerPrev: new Map(), randomStates,
            lastRandom: '', active: true
        };
    }

    function evaluateDialogueClauses(config, runtime = dialogueRuntime) {
        const clauses = config && Array.isArray(config.clauses) ? config.clauses : [];
        if(clauses.length === 0) return false;
        const results = clauses.map(clause => {
            const left = Number(runtime.variables.get(sanitizeIniVarToken(clause.variable, '')) || 0); const right = Number(clause.value) || 0;
            if(clause.operator === '!=') return left !== right; if(clause.operator === '>') return left > right;
            if(clause.operator === '>=') return left >= right; if(clause.operator === '<') return left < right;
            if(clause.operator === '<=') return left <= right; return left === right;
        });
        return config.mode === 'any' ? results.some(Boolean) : results.every(Boolean);
    }

    function getGraphOutgoing(graph, nodeId, port = null) {
        return (graph.edges || []).filter(edge => edge.fromNodeId === nodeId && (port == null || edge.fromPort === port));
    }

    function setDialogueTextVisible(component, visible) {
        if(!component || component.type !== 'text') return;
        previewTextVariableStates.set(getTextVisibilityVar(component), !!visible);
        const state = dialogueRuntime.textStates.get(component.id) || { lastTrigger: -Infinity, expiresAt: 0, visible: false };
        state.visible = !!visible;
        if(!visible) state.expiresAt = 0;
        dialogueRuntime.textStates.set(component.id, state);
    }

    function activateDialogueText(componentId, context = {}) {
        const component = getComponentById(componentId); if(!component || component.type !== 'text') return false;
        const state = dialogueRuntime.textStates.get(component.id) || { lastTrigger: -Infinity, expiresAt: 0, visible: false };
        const cooldown = Math.max(0, Number(component.triggerCooldownSeconds) || 0);
        if(Number.isFinite(state.lastTrigger) && dialogueRuntime.clock - state.lastTrigger < cooldown) return false;
        state.lastTrigger = dialogueRuntime.clock; state.visible = true; state.context = cloneDeep(context);
        const lifetime = Math.max(0, Number(component.lifetimeSeconds) || 0);
        state.expiresAt = lifetime > 0 ? dialogueRuntime.clock + lifetime : 0;
        dialogueRuntime.textStates.set(component.id, state);
        previewTextVariableStates.set(getTextVisibilityVar(component), true);
        return true;
    }

    function closeDialogueStep(dialogue, stepNode) {
        if(!dialogue || !stepNode) return;
        (stepNode.config && stepNode.config.textIds || []).forEach(id => setDialogueTextVisible(getComponentById(id), false));
        const state = dialogueRuntime.dialogueStates.get(dialogue.id);
        if(state) state.stepNodeId = '';
    }

    function activateDialogueStep(dialogue, stepNode) {
        if(!dialogue || !stepNode) return;
        const previous = dialogueRuntime.dialogueStates.get(dialogue.id);
        if(previous && previous.stepNodeId) closeDialogueStep(dialogue, dialogue.nodes.find(node => node.id === previous.stepNodeId));
        const state = previous || { active: true, stepNodeId: '' };
        state.active = true; state.stepNodeId = stepNode.id; dialogueRuntime.dialogueStates.set(dialogue.id, state);
        let accepted = 0;
        (stepNode.config && stepNode.config.textIds || []).forEach(id => { if(activateDialogueText(id, { dialogueId: dialogue.id, stepNodeId: stepNode.id })) accepted++; });
        if(accepted === 0) { state.active = false; state.stepNodeId = ''; }
    }

    function endDialogue(dialogue) {
        if(!dialogue) return;
        const state = dialogueRuntime.dialogueStates.get(dialogue.id);
        if(state && state.stepNodeId) closeDialogueStep(dialogue, dialogue.nodes.find(node => node.id === state.stepNodeId));
        dialogueRuntime.dialogueStates.set(dialogue.id, { active: false, stepNodeId: '' });
    }

    function applyDialogueAssignments(assignments) {
        (assignments || []).forEach(action => {
            const name = sanitizeIniVarToken(action.variable, ''); if(!name) return;
            const current = Number(dialogueRuntime.variables.get(name) || 0), value = Number(action.value) || 0;
            if(action.operation === 'add') dialogueRuntime.variables.set(name, current + value);
            else if(action.operation === 'subtract') dialogueRuntime.variables.set(name, current - value);
            else if(action.operation === 'toggle') dialogueRuntime.variables.set(name, 1 - (current ? 1 : 0));
            else dialogueRuntime.variables.set(name, value);
        });
    }

    function advanceRandomState(node) {
        let value = Number(dialogueRuntime.randomStates.get(node.id));
        if(!(value > 0 && value < 1)) value = Number(node.config && node.config.seed) || .3187;
        value = 3.91 * value * (1 - value);
        dialogueRuntime.randomStates.set(node.id, value);
        return value;
    }

    function dispatchDialogueNode(graph, nodeId, context = {}, guard = 0) {
        if(guard > 64) throw new Error('蓝图即时执行超过 64 步，请检查循环。');
        const node = (graph.nodes || []).find(item => item.id === nodeId); if(!node) return;
        const cfg = node.config || {};
        if(node.type === 'text') { activateDialogueText(cfg.componentId, { graphScope: graph.id || 'main', nodeId: node.id }); return; }
        if(node.type === 'dialogue') {
            const dialogue = getDialogueDefinitionForGroup(cfg.groupId); if(!dialogue) return;
            const state = dialogueRuntime.dialogueStates.get(dialogue.id); if(state && state.active) return;
            dialogueRuntime.dialogueStates.set(dialogue.id, { active: true, stepNodeId: '' });
            dispatchDialogueNode(dialogue, dialogue.entryNodeId, { dialogueId: dialogue.id }, guard + 1); return;
        }
        if(node.type === 'step') { activateDialogueStep(graph, node); return; }
        if(node.type === 'exit') { if(graph !== dialogueLogic.main) endDialogue(graph); return; }
        if(node.type === 'action') applyDialogueAssignments(cfg.assignments);
        let port = 'out';
        if(node.type === 'condition') port = evaluateDialogueClauses(cfg) ? 'true' : 'false';
        if(node.type === 'random') {
            const value = advanceRandomState(node); const edges = getGraphOutgoing(graph, node.id); let cumulative = 0, chosen = edges[edges.length - 1];
            for(const edge of edges) { cumulative += Math.max(0, Number(edge.weightBp) || 0) / 10000; if(value < cumulative) { chosen = edge; break; } }
            if(chosen) { dialogueRuntime.lastRandom = `${getBlueprintNodeLabel(node)} → ${(Number(chosen.weightBp)||0)/100}%`; dispatchDialogueNode(graph, chosen.toNodeId, context, guard + 1); }
            return;
        }
        const edge = getGraphOutgoing(graph, node.id, port)[0] || (port === 'out' ? getGraphOutgoing(graph, node.id)[0] : null);
        if(edge) dispatchDialogueNode(graph, edge.toNodeId, context, guard + 1);
    }

    function dispatchDirectTextRandom(componentId) {
        const ref = getTextRandomNode(componentId); if(!ref || !(ref.node.config.branches || []).length) return false;
        const value = advanceRandomState(ref.node); let cumulative = 0, chosen = ref.node.config.branches[ref.node.config.branches.length - 1];
        for(const branch of ref.node.config.branches) { cumulative += Math.max(0, Number(branch.weightBp) || 0) / 10000; if(value < cumulative) { chosen = branch; break; } }
        if(chosen && chosen.targetTextId) {
            dialogueRuntime.lastRandom = `${getEntityLabel(makeEntityRef('component', componentId))} → ${getEntityLabel(makeEntityRef('component', chosen.targetTextId))}`;
            const target = getComponentById(chosen.targetTextId);
            const targetState = dialogueRuntime.textStates.get(chosen.targetTextId);
            if(targetState && targetState.visible) setDialogueTextVisible(target, false);
            else activateDialogueText(chosen.targetTextId);
        }
        return true;
    }

    function handleDialogueTextEvent(componentId, eventType) {
        if(!dialogueRuntime) return false;
        const state = dialogueRuntime.textStates.get(componentId); if(!state || !state.visible) return false;
        const membership = getComponentDialogueMembership(componentId);
        if(eventType === 'click' && getTextRandomNode(componentId)) {
            if(membership) closeDialogueStep(membership.dialogue, membership.step);
            dispatchDirectTextRandom(componentId); return true;
        }
        if(membership) {
            const port = `${eventType}:${componentId}`; const edge = getGraphOutgoing(membership.dialogue, membership.step.id, port)[0];
            if(edge) { closeDialogueStep(membership.dialogue, membership.step); dispatchDialogueNode(membership.dialogue, edge.toNodeId, { dialogueId: membership.dialogue.id }); return true; }
            if(eventType === 'timeout') {
                setDialogueTextVisible(getComponentById(componentId), false);
                const anyVisible = (membership.step.config.textIds || []).some(id => dialogueRuntime.textStates.get(id) && dialogueRuntime.textStates.get(id).visible);
                if(!anyVisible) endDialogue(membership.dialogue);
            }
            return false;
        }
        const textNode = dialogueLogic.main.nodes.find(node => node.type === 'text' && node.config && node.config.componentId === componentId);
        const edge = textNode && getGraphOutgoing(dialogueLogic.main, textNode.id, eventType)[0];
        if(edge) { setDialogueTextVisible(getComponentById(componentId), false); dispatchDialogueNode(dialogueLogic.main, edge.toNodeId); return true; }
        if(eventType === 'timeout') setDialogueTextVisible(getComponentById(componentId), false);
        return false;
    }

    function tickDialogueRuntime() {
        if(!dialogueRuntime || workspaceMode !== 'run') return;
        const now = performance.now() / 1000, delta = Math.max(0, Math.min(.25, now - dialogueRuntime.lastReal)); dialogueRuntime.lastReal = now;
        if(!dialogueRuntime.paused) {
            dialogueRuntime.clock += delta;
            [...dialogueRuntime.textStates.entries()].forEach(([id, state]) => { if(state.visible && state.expiresAt > 0 && dialogueRuntime.clock >= state.expiresAt) handleDialogueTextEvent(id, 'timeout'); });
            (dialogueLogic.main.nodes || []).filter(node => node.type === 'trigger').forEach(node => {
                const current = evaluateDialogueClauses(node.config || {}), previous = dialogueRuntime.triggerPrev.get(node.id) === true;
                if(current && !previous) { const edge = getGraphOutgoing(dialogueLogic.main, node.id)[0]; if(edge) dispatchDialogueNode(dialogueLogic.main, edge.toNodeId); }
                dialogueRuntime.triggerPrev.set(node.id, current);
            });
        }
        renderRuntimeInspector(); renderAll();
        dialogueRuntimeFrame = requestAnimationFrame((__ts) => { if (!root.isConnected) return; tickDialogueRuntime(__ts); });
    }

    function startDialoguePreview() {
        if(workspaceMode === 'run') { stopDialoguePreview(true); return; }
        const errors = validateDialogueLogic(); if(errors.length) { alert(`无法运行文本蓝图：\n${errors.join('\n')}`); return; }
        workspaceMode = 'run'; dialogueRuntime = createDialogueRuntime(); previewTextVariableStates.clear();
        components.filter(component => component.type === 'text' && component.textVisibilityEnabled).forEach(component => previewTextVariableStates.set(getTextVisibilityVar(component), component.visDefault !== false));
        root.classList.add('preview-running'); document.getElementById('blueprintWorkspace').style.display = 'none'; workArea.style.display = 'block';
        document.getElementById('runtimeInspector').style.display = 'block'; syncWorkspaceModeButtons(); renderAll();
        if(dialogueRuntimeFrame) cancelAnimationFrame(dialogueRuntimeFrame); dialogueRuntimeFrame = requestAnimationFrame((__ts) => { if (!root.isConnected) return; tickDialogueRuntime(__ts); });
    }

    function stopDialoguePreview(returnToLayout = false) {
        if(dialogueRuntimeFrame) cancelAnimationFrame(dialogueRuntimeFrame); dialogueRuntimeFrame = null;
        if(workspaceMode === 'run' || dialogueRuntime) { dialogueRuntime = null; previewTextVariableStates.clear(); root.classList.remove('preview-running'); }
        if(returnToLayout) { workspaceMode = 'layout'; document.getElementById('runtimeInspector').style.display = 'none'; workArea.style.display = 'block'; syncWorkspaceModeButtons(); renderAll(); }
    }

    window.toggleDialoguePreviewPause = () => { if(dialogueRuntime) { dialogueRuntime.paused = !dialogueRuntime.paused; dialogueRuntime.lastReal = performance.now() / 1000; renderRuntimeInspector(); } };
    window.restartDialoguePreview = () => { if(workspaceMode === 'run') { dialogueRuntime = createDialogueRuntime(); previewTextVariableStates.clear(); renderRuntimeInspector(); renderAll(); } };
    window.setDialoguePreviewVariable = (name, value) => { if(dialogueRuntime) dialogueRuntime.variables.set(name, Number(value) || 0); };

    function renderRuntimeInspector() {
        const wrap = document.getElementById('runtimeInspector'); if(!wrap || !dialogueRuntime) return;
        const active = [...dialogueRuntime.dialogueStates.entries()].filter(([,state]) => state.active).map(([id,state]) => { const d = dialogueLogic.dialogues.find(item => item.id === id); const step = d && d.nodes.find(node => node.id === state.stepNodeId); return `${d ? d.name : id}: ${step ? getBlueprintNodeLabel(step) : '启动中'}`; });
        const textRows = [...dialogueRuntime.textStates.entries()].filter(([,state]) => state.visible).map(([id,state]) => { const comp = getComponentById(id); const remain = state.expiresAt > 0 ? Math.max(0, state.expiresAt - dialogueRuntime.clock).toFixed(1) + 's' : '持续'; return `<div class="anim-note">${escapeHtml(getEntityLabel(makeEntityRef('component', id)))} · ${remain}</div>`; }).join('');
        wrap.innerHTML = `<div style="display:flex;gap:5px"><button onclick="UIB.toggleDialoguePreviewPause()">${dialogueRuntime.paused ? '继续' : '暂停'}</button><button onclick="UIB.restartDialoguePreview()">重新开始</button><button onclick="UIB.setWorkspaceMode('run')">停止</button></div><h4>运行状态 · ${dialogueRuntime.clock.toFixed(1)}s</h4><div class="anim-note">${escapeHtml(active.join('；') || '没有活动对话')}<br>${escapeHtml(dialogueRuntime.lastRandom || '尚未触发随机分支')}</div>${textRows}<h4 style="margin-top:10px">变量检查器</h4>` + [...dialogueRuntime.variables.entries()].map(([name,value]) => `<div class="runtime-row"><span class="var-tag">${escapeHtml(name)}</span><input type="number" step="any" value="${Number(value)}" onchange="UIB.setDialoguePreviewVariable('${escapeHtml(name)}',this.value)"></div>`).join('');
    }

    function getComponentsAtClientPoint(clientX, clientY) {
        const orderMap = new Map(components.map((component, index) => [component.id, index]));
        return components
            .filter(component => {
                const el = document.getElementById(component.id);
                if(!el) return false;
                const rect = el.getBoundingClientRect();
                return clientX >= rect.left && clientX <= rect.right && clientY >= rect.top && clientY <= rect.bottom;
            })
            .sort((a, b) => {
                const az = Number.isFinite(Number(a.zIndex)) ? Number(a.zIndex) : 0;
                const bz = Number.isFinite(Number(b.zIndex)) ? Number(b.zIndex) : 0;
                if(az !== bz) return bz - az;
                return (orderMap.get(b.id) ?? -1) - (orderMap.get(a.id) ?? -1);
            });
    }

    function cycleSelectionAtPoint(clientX, clientY, isMulti = false) {
        const hits = getComponentsAtClientPoint(clientX, clientY);
        if(hits.length === 0) return false;
        const currentIdx = hits.findIndex(component => component.id === selectedId);
        const next = currentIdx >= 0 ? hits[(currentIdx + 1) % hits.length] : hits[0];
        selectItem(next.id, isMulti);
        return true;
    }

    function isEditableEventTarget(target) {
        if(!target) return false;
        const el = target.closest ? target.closest('input, textarea, select, [contenteditable="true"]') : null;
        return !!el;
    }

    function shouldTrackHistoryEvent(e) {
        if(isRestoringHistory) return false;
        const target = e.target;
        if(!target || !target.closest) return true;
        if(target.closest('#modal')) return false;
        if(target.closest('#undo_btn, #redo_btn')) return false;
        return true;
    }

    __onWindow('input', (e) => {
        if(shouldTrackHistoryEvent(e)) markHistoryDirty();
    }, true);

    __onWindow('change', (e) => {
        if(shouldTrackHistoryEvent(e)) markHistoryDirty();
    }, true);

    __onWindow('click', (e) => {
        if(e.target && e.target.closest && e.target.closest('button') && shouldTrackHistoryEvent(e)) {
            markHistoryDirty();
        }
    }, true);

    function onHistoryTrackedPointerDown(e) {
        if(e.button !== 0 || isRestoringHistory) return;
        if(e.target && e.target.closest && e.target.closest('#modal')) return;
        if(isEditableEventTarget(e.target)) markHistoryDirty();
    }

    __onWindow('pointerdown', onHistoryTrackedPointerDown, true);
    __onWindow('mousedown', onHistoryTrackedPointerDown, true);

    workArea.addEventListener('mousedown', (e) => {
        if(e.target !== workArea) return;
        dragSession = null;
        clearSelection();
        propPanel.style.display = 'none';
        if(componentPanelWrap) componentPanelWrap.style.display = 'none';
        resize();
        refreshAnimationPanelVisibility(null);
        renderHierarchyPanel();
        renderAll();
    });
    workArea.addEventListener('mousemove', (e) => {
        const point = normalizeCursorPoint(e.clientX, e.clientY);
        previewPointerState.x = point.x;
        previewPointerState.y = point.y;
        previewPointerState.inside = true;
    });
    workArea.addEventListener('mouseenter', (e) => {
        const point = normalizeCursorPoint(e.clientX, e.clientY);
        previewPointerState.x = point.x;
        previewPointerState.y = point.y;
        previewPointerState.inside = true;
    });
    workArea.addEventListener('mouseleave', () => {
        previewPointerState.inside = false;
        setDockGuide('');
    });
    __onWindow('keydown', (e) => {
      if (!root.isConnected) return;
        if(e.key === 'Alt') previewPointerState.alt = true;
        if(e.defaultPrevented || isEditableEventTarget(e.target)) return;
        const key = String(e.key || '').toLowerCase();
        const isMod = e.ctrlKey || e.metaKey;
        if(isMod && !e.altKey && key === 'z') {
            e.preventDefault();
            if(e.shiftKey) redoHistory();
            else undoHistory();
            return;
        }
        if(isMod && !e.altKey && key === 'y') {
            e.preventDefault();
            redoHistory();
            return;
        }
        if((e.key === 'Delete' || e.key === 'Del') && !isMod && !e.altKey) {
            if(workspaceMode === 'blueprint' && selectedBlueprintNodeIds.length > 0) {
                e.preventDefault();
                deleteSelectedBlueprintItems();
                return;
            }
            if(selectedEntity) {
                e.preventDefault();
                deleteItem();
            }
        }
    });
    __onWindow('keyup', (e) => {
      if (!root.isConnected) return;
        if(e.key === 'Alt') {
            previewPointerState.alt = false;
            setDockGuide('');
        }
    });
    __onWindow('blur', () => {
      if (!root.isConnected) return;
        previewPointerState.alt = false;
        setDockGuide('');
    });

    function updatePropPanel() {
        const group = selectedGroup();
        let obj = selectedObj();
        const groupPanel = document.getElementById('group_prop_panel');
        const componentPanel = document.getElementById('component_prop_panel');
        const componentNotice = document.getElementById('component_group_notice');
        if(!group && !obj) {
            refreshTextLogicWindow(null);
            refreshResourceWindow(null);
            propPanel.style.display = 'none';
            if(componentPanelWrap) componentPanelWrap.style.display = 'none';
            resize();
            refreshAnimationPanelVisibility(null);
            return;
        }
        propPanel.style.display = 'block';
        if(componentPanelWrap) componentPanelWrap.style.display = 'block';
        resize();
        refreshAnimationPanelVisibility(obj);
        refreshResourceWindow(obj);
        refreshTextLogicWindow(obj);

        if(groupPanel) groupPanel.style.display = group ? 'block' : 'none';
        if(componentPanel) componentPanel.style.display = obj ? 'block' : 'none';

        if(group) {
            refreshTextLogicWindow(null);
            document.getElementById('g_name').value = sanitizeGroupDisplayName(group.name, group.id);
            document.getElementById('g_vis_var').value = group.visVar || '';
            document.getElementById('g_vis_default').checked = group.visDefault !== false;
            document.getElementById('g_pinned').checked = group.pinned === true;
            document.getElementById('g_binding_enabled').checked = group.bindingEnabled !== false;
            const suffix = getGroupRuntimeVarSuffix(group.id);
            document.getElementById('g_vis_var_name').innerText = group.visVar || `$grp_show_${suffix}`;
            document.getElementById('g_pin_var_name').innerText = `$grp_pin_mode_${suffix}`;
            document.getElementById('g_bind_var_name').innerText = `$grp_bind_mode_${suffix}`;
            applyAnimationEditorState(null);
            return;
        }

        let objIndex = components.findIndex(c => c.id === obj.id);

        refreshGeomInputs(obj);
        document.getElementById('p_z').value = obj.zIndex || 0;
        document.getElementById('p_rot').value = obj.rot || 0;
        document.getElementById('p_corner_radius').value = Number(getComponentCornerRadiusPx(obj).toFixed(2));
        const ownerGroup = getOwningGroupForComponent(obj);
        if(componentNotice) {
            if(ownerGroup) {
                componentNotice.style.display = 'block';
                componentNotice.innerHTML = obj.type === 'text'
                    ? `当前文本框位于编组 <strong>${ownerGroup.name || ownerGroup.id}</strong> 中。绘制时先判断编组显示，再判断文本框独立显示，两者互不覆盖。`
                    : `当前组件受编组 <strong>${ownerGroup.name || ownerGroup.id}</strong> 控制，显示、默认固定、参数绑定和全局动画均由编组统一决定。`;
            } else {
                componentNotice.style.display = 'none';
                componentNotice.innerHTML = '';
            }
        }

        const logicWrap = document.getElementById('logic_wrapper');
        const visGroup = document.getElementById('vis_group');
        const rowPhys = document.getElementById('row_phys');
        const rowFollowCursor = document.getElementById('row_follow_cursor');
        const rowFollowOffset = document.getElementById('row_follow_offset');
        const followCursorHint = document.getElementById('follow_cursor_hint');
        const rowZoneDrag = document.getElementById('row_zone_drag');
        const rowZoneDragVar = document.getElementById('row_zone_drag_var');
        const rowZoneDragId = document.getElementById('row_zone_drag_id');
        const zoneDragHint = document.getElementById('zone_drag_hint');
        const textGroup = document.getElementById('text_editor_group');
        const seqGroup = document.getElementById('seq_editor_group');
        const rowSwitchGroup = document.getElementById('row_switch_group');
        const rowToggleSteps = document.getElementById('row_toggle_steps');
        const rowGridSteps = document.getElementById('row_grid_steps');
        const rowGridValueStart = document.getElementById('row_grid_value_start');
        const rowGridValueStep = document.getElementById('row_grid_value_step');
        const rowSliderSubdiv = document.getElementById('row_slider_subdiv');
        const sliderSubdivLabel = document.getElementById('slider_subdiv_label');
        const sliderSubdivHint = document.getElementById('slider_subdiv_hint');
        const rowJoyDirCount = document.getElementById('row_joy_dir_count');
        const rowJoySubdiv = document.getElementById('row_joy_subdiv');
        const rowJoyAngleOffset = document.getElementById('row_joy_angle_offset');
        const rowJoyDefaultX = document.getElementById('row_joy_default_x');
        const rowJoyDefaultY = document.getElementById('row_joy_default_y');
        const joyAlgoHint = document.getElementById('joy_algo_hint');
        const rowPhysProfile = document.getElementById('row_phys_profile');
        const physProfileHint = document.getElementById('phys_profile_hint');
        const physConfigPanel = document.getElementById('phys_config_panel');
        const rowAutoSource = document.getElementById('row_auto_source');
        const rowAutoAmpX = document.getElementById('row_auto_amp_x');
        const rowAutoAmpY = document.getElementById('row_auto_amp_y');
        const lblAutoAmpX = document.getElementById('lbl_auto_amp_x');
        const rowAutoSeedX = document.getElementById('row_auto_seed_x');
        const rowAutoSeedY = document.getElementById('row_auto_seed_y');
        const lblAutoSeedX = document.getElementById('lbl_auto_seed_x');
        const btnAddVar = document.getElementById('btn_add_var');
        const accumPanel = document.getElementById('accum_panel');
        const rowInitial = document.getElementById('row_initial_val');
        const rowToggleInvert = document.getElementById('row_toggle_invert');
        const rowInitialNumber = document.getElementById('row_initial_num');

        textGroup.style.display = 'none';
        seqGroup.style.display = 'none';
        logicWrap.style.display = 'none';
        visGroup.style.display = 'none';
        rowPhys.style.display = 'none';
        rowFollowCursor.style.display = 'none';
        rowFollowOffset.style.display = 'none';
        followCursorHint.style.display = 'none';
        rowZoneDrag.style.display = 'none';
        rowZoneDragVar.style.display = 'none';
        rowZoneDragId.style.display = 'none';
        zoneDragHint.style.display = 'none';
        rowSwitchGroup.style.display = 'none';
        rowToggleSteps.style.display = 'none';
        rowGridSteps.style.display = 'none';
        rowGridValueStart.style.display = 'none';
        rowGridValueStep.style.display = 'none';
        rowSliderSubdiv.style.display = 'none';
        sliderSubdivHint.style.display = 'none';
        rowJoyDirCount.style.display = 'none';
        rowJoySubdiv.style.display = 'none';
        rowJoyAngleOffset.style.display = 'none';
        rowJoyDefaultX.style.display = 'none';
        rowJoyDefaultY.style.display = 'none';
        joyAlgoHint.style.display = 'none';
        rowPhysProfile.style.display = 'none';
        physProfileHint.style.display = 'none';
        physConfigPanel.style.display = 'none';
        rowAutoSource.style.display = 'flex';
        rowAutoAmpX.style.display = 'flex';
        rowAutoAmpY.style.display = 'flex';
        rowAutoSeedX.style.display = 'flex';
        rowAutoSeedY.style.display = 'flex';
        lblAutoAmpX.innerText = '幅度范围:';
        btnAddVar.style.display = 'none';
        btnAddVar.innerText = '+ 添加变量';
        lblAutoSeedX.innerText = '随机种子:';
        rowInitial.style.display = 'none';
        rowToggleInvert.style.display = 'none';
        rowInitialNumber.style.display = 'none';
        accumPanel.style.display = 'none';
        document.getElementById('auto_var_name').innerText = '';
        applyAnimationEditorState(obj);

        if(obj.type === 'text') {
            textGroup.style.display = 'block';
            document.getElementById('p_text_content').value = obj.textContent || '';
            document.getElementById('p_char_size').value = obj.charSize || 0.03;
            document.getElementById('p_line_gap').value = obj.lineGap || 0.0;
            document.getElementById('p_val_var').value = obj.valVar || '';
            
            document.getElementById('p_font_family').value = obj.fontFamily || 'Microsoft YaHei';
            document.getElementById('p_font_color').value = obj.fontColor || '#ffffff';
            document.getElementById('p_font_bold').checked = obj.fontBold;
            document.getElementById('p_font_italic').checked = obj.fontItalic;
            document.getElementById('p_text_flow').value = obj.textFlow || 'horizontal';
            document.getElementById('p_text_visibility_enabled').checked = obj.textVisibilityEnabled === true;
            document.getElementById('p_text_vis_var').value = getTextVisibilityVar(obj);
            document.getElementById('p_text_vis_var').disabled = obj.textVisibilityEnabled !== true;
            document.getElementById('p_text_vis_var_name').innerText = obj.textVisibilityEnabled === true ? getTextVisibilityVar(obj) : '未启用';
            document.getElementById('p_text_vis_default').checked = obj.visDefault !== false;
            document.getElementById('p_text_vis_default').disabled = obj.textVisibilityEnabled !== true;
            document.getElementById('p_text_hover_effect').checked = obj.textHoverEffect === true;
            document.getElementById('p_text_click_var').value = getTextClickVar(obj);
            
        } else if (obj.type === 'sequence') {
            seqGroup.style.display = 'block';
            document.getElementById('p_seq_var').value = obj.seqVar || '';
            renderSeqList(obj);
            rowFollowCursor.style.display = 'flex';
            rowFollowOffset.style.display = obj.followCursor === true ? 'flex' : 'none';
            followCursorHint.style.display = 'block';
            document.getElementById('p_follow_cursor').checked = obj.followCursor === true;
            document.getElementById('p_follow_offset_x').value = Number(obj.followOffsetX ?? 0.5);
            document.getElementById('p_follow_offset_y').value = Number(obj.followOffsetY ?? 0.5);
        } else if (obj.type === 'static') {
            // Static images only expose geometry and resources.
            rowFollowCursor.style.display = 'flex';
            rowFollowOffset.style.display = obj.followCursor === true ? 'flex' : 'none';
            followCursorHint.style.display = 'block';
            document.getElementById('p_follow_cursor').checked = obj.followCursor === true;
            document.getElementById('p_follow_offset_x').value = Number(obj.followOffsetX ?? 0.5);
            document.getElementById('p_follow_offset_y').value = Number(obj.followOffsetY ?? 0.5);
        } else {
            // Interactive Widgets
            logicWrap.style.display = 'block';
            const genericVarBlock = document.getElementById('generic_var_block');
            if(genericVarBlock) genericVarBlock.style.display = 'block';
            const rowMode = document.getElementById('row_mode');
            if(obj.type==='toggle') {
                rowMode.style.display = 'flex';
                updateModeSelect(obj.type);
                document.getElementById('p_mode').value = obj.paramMode || '1';
                rowSwitchGroup.style.display = 'flex';
                document.getElementById('p_switch_group').value = obj.switchGroup || 0;
                btnAddVar.style.display = 'flex'; 
                if(isToggleMultiMode(obj)) {
                    rowToggleSteps.style.display = 'flex';
                    document.getElementById('p_toggle_steps').value = obj.toggleSteps || DEFAULT_TOGGLE_STEPS;
                    rowInitialNumber.style.display = 'flex';
                    document.getElementById('p_initial_num').max = obj.toggleSteps || DEFAULT_TOGGLE_STEPS;
                    document.getElementById('p_initial_num').value = obj.initialValue || 0;
                } else {
                    rowInitial.style.display = 'flex';
                    document.getElementById('p_initial_val').checked = (obj.initialValue === 1);
                    rowToggleInvert.style.display = 'flex';
                    document.getElementById('p_toggle_invert').checked = obj.toggleInvert === true;
                }
            }
            else if(obj.type === 'accum') {
                accumPanel.style.display = 'block';
                if(genericVarBlock) genericVarBlock.style.display = 'none';
                document.getElementById('row_mode').style.display = 'none';
                const rowHandleSize = document.getElementById('row_handle_size');
                if(rowHandleSize) rowHandleSize.style.display = 'none';
                document.getElementById('p_accum_direction').value = obj.accumDirection === 'v' ? 'v' : 'h';
                document.getElementById('p_accum_threshold').value = obj.accumThreshold;
                renderAccumBindings();
                renderAccumTriggers();
            }
            else { 
                rowMode.style.display = 'flex'; updateModeSelect(obj.type); document.getElementById('p_mode').value = obj.paramMode; 
                if(obj.type.includes('slider') && obj.paramMode === '3') {
                    normalizeSliderGridState(obj);
                    rowGridSteps.style.display = 'flex';
                    rowGridValueStart.style.display = 'flex';
                    rowGridValueStep.style.display = 'flex';
                    document.getElementById('p_grid_steps').value = obj.gridSteps || 3;
                    document.getElementById('p_grid_value_start').value = getSliderGridValueStart(obj);
                    document.getElementById('p_grid_value_step').value = getSliderGridValueStep(obj);
                    btnAddVar.style.display = 'flex';
                    btnAddVar.innerText = '+ 添加绑定变量';
                } else {
                    rowGridSteps.style.display = 'none';
                    rowGridValueStart.style.display = 'none';
                    rowGridValueStep.style.display = 'none';
                    btnAddVar.style.display = 'flex';
                    btnAddVar.innerText = '+ 添加变量(槽0)';
                }

                if(obj.type.includes('slider') && (obj.paramMode === '1' || obj.paramMode === '2')) {
                    const sliderCfg = getSliderSubdivisionConfig(obj);
                    rowSliderSubdiv.style.display = 'flex';
                    sliderSubdivHint.style.display = sliderCfg.subdivisions > 1 ? 'block' : 'none';
                    sliderSubdivLabel.textContent = obj.paramMode === '2' ? '每侧细分:' : '行程细分:';
                    document.getElementById('p_slider_subdiv').value = sliderCfg.subdivisions;
                    if(sliderCfg.subdivisions > 1) btnAddVar.style.display = 'none';
                }

                if(obj.type === 'joystick' && obj.paramMode === '4') {
                    const joyCfg = getJoystickConfig(obj);
                    rowJoyDirCount.style.display = 'flex';
                    rowJoySubdiv.style.display = 'flex';
                    rowJoyAngleOffset.style.display = 'flex';
                    joyAlgoHint.style.display = 'block';
                    document.getElementById('p_joy_dir_count').value = joyCfg.directionCount;
                    document.getElementById('p_joy_subdiv').value = joyCfg.subdivisions;
                    document.getElementById('p_joy_angle_offset').value = joyCfg.angleOffset;
                }
                // 所有摇杆类型都显示默认位置输入
                if(obj.type === 'joystick') {
                    rowJoyDefaultX.style.display = 'flex';
                    rowJoyDefaultY.style.display = 'flex';
                    document.getElementById('p_joy_default_x').value = Number(obj.joystickDefaultX ?? 0).toFixed(2);
                    document.getElementById('p_joy_default_y').value = Number(obj.joystickDefaultY ?? 0).toFixed(2);
                }

                // 模型区域拖拽：滑条/摇杆可改用“命中模型指定区域”触发手柄拖拽
                if(componentSupportsZoneDrag(obj)) {
                    const zoneDragOn = obj.zoneDragEnabled === true;
                    rowZoneDrag.style.display = 'flex';
                    rowZoneDragVar.style.display = zoneDragOn ? 'flex' : 'none';
                    rowZoneDragId.style.display = zoneDragOn ? 'flex' : 'none';
                    zoneDragHint.style.display = zoneDragOn ? 'block' : 'none';
                    document.getElementById('p_zone_drag_enabled').checked = zoneDragOn;
                    document.getElementById('p_zone_drag_var').value = getZoneDragVarName(obj);
                    document.getElementById('p_zone_drag_id').value = getZoneDragZoneId(obj);
                }
            }

            visGroup.style.display = (obj.type==='toggle') ? 'none' : 'block';
            const rowHandleSizeEl = document.getElementById('row_handle_size');
            if(rowHandleSizeEl) rowHandleSizeEl.style.display = (obj.type === 'accum') ? 'none' : 'flex';
            if(obj.type!=='toggle') {
                document.getElementById('p_hs').value = obj.handleSize;
                document.getElementById('p_tt').value = obj.trackThick;
            }

            const physicsSupported = componentSupportsPhysics(obj);
            if(!physicsSupported) obj.physics = false;

            if(physicsSupported) {
                rowPhys.style.display = 'flex';
                const pBox = document.getElementById('p_phys');
                const pLbl = document.getElementById('phys_label');
                const pVar = document.getElementById('phys_var_name');
                pBox.checked = (obj.physics === true);
                pBox.disabled = false;
                pVar.innerText = `$phys_mode_${objIndex}`;
                pLbl.innerText = obj.physics ? '物理默认开启' : '物理默认关闭';
                pLbl.style.color = obj.physics ? '#4dedb4' : '#aaa';

                if(obj.physics) {
                    physConfigPanel.style.display = 'block';
                    document.getElementById('p_spring_k_val').value = obj.springK ?? 0.05;
                    document.getElementById('p_spring_d_val').value = obj.springD ?? 0.95;
                    document.getElementById('p_phys_profile').value = obj.physicsProfile || 'normal';
                    document.getElementById('p_auto_animate').checked = obj.autoAnimate || false;
                    document.getElementById('auto_var_name').innerText = `$auto_${objIndex}`;
                    document.getElementById('p_auto_str').value = obj.autoStr ?? 0.1;
                    document.getElementById('p_auto_source').value = getAutoSourceMode(obj);
                    document.getElementById('p_auto_amp_x').value = obj.autoAmpX ?? 1;
                    document.getElementById('p_auto_amp_y').value = obj.autoAmpY ?? 1;
                    document.getElementById('p_auto_seed_x').value = obj.autoSeedX ?? 0.3187;
                    document.getElementById('p_auto_seed_y').value = obj.autoSeedY ?? 0.6123;
                    document.getElementById('p_auto_func_x').value = obj.autoFuncX || (obj.type === 'joystick' ? 'sin(TAU * t)' : 'sin01(t)');
                    document.getElementById('p_auto_func_y').value = obj.autoFuncY || 'cos(TAU * t)';
                    document.getElementById('p_auto_speed').value = obj.autoSpeed ?? 0.015;
                    document.getElementById('p_auto_response').value = obj.autoResponse ?? 0.22;
                    document.getElementById('p_auto_bounce').value = obj.autoBounce ?? 0.25;
                    document.getElementById('p_gravity').value = obj.gravity ?? 0;
                    document.getElementById('p_chaos_rate').value = obj.chaosRate || 96;
                    if(obj.type === 'joystick') {
                        rowPhysProfile.style.display = 'flex';
                        physProfileHint.style.display = (obj.physicsProfile === 'breast') ? 'block' : 'none';
                    }
                    if(obj.type.includes('slider')) {
                        lblAutoAmpX.innerText = '自动范围:';
                        rowAutoAmpY.style.display = 'none';
                        lblAutoSeedX.innerText = '轨迹种子:';
                        rowAutoSeedY.style.display = 'none';
                    }
                    applyAutoEditorState(obj);
                }
            }

            // 嵌套联动面板：滑块和摇杆可配置
            const linkedSlavesPanel = document.getElementById('linked_slaves_panel');
            if(obj.type.includes('slider') || obj.type === 'joystick') {
                linkedSlavesPanel.style.display = 'block';
                renderLinkedSlaves();
                renderRangeTriggers();
            } else {
                linkedSlavesPanel.style.display = 'none';
            }

            renderVarInputs(obj);
            return;

            if(obj.type === 'toggle') {
                rowPhys.style.display = 'none';
            }
            else if(componentSupportsPhysics(obj)) {
                rowPhys.style.display = 'flex';
                const pBox = document.getElementById('p_phys');
                const pLbl = document.getElementById('phys_label');
                const pVar = document.getElementById('phys_var_name');
                pBox.checked = (obj.physics === true);
                pBox.disabled = false;
                pVar.innerText = `$phys_mode_${objIndex}`;
                
                if(obj.type.includes('slider') && (obj.paramMode === '1' || obj.paramMode === '3')) {
                    pBox.disabled = true; pBox.checked = false; obj.physics = false;
                    pLbl.innerText = "Unavailable (only split joystick)"; pLbl.style.color = "#666";
                    pVar.innerText = "";
                    physConfigPanel.style.display = 'none';
                } else {
                    pLbl.innerText = obj.physics ? "物理默认开启" : "物理默认关闭";
                    pLbl.style.color = obj.physics ? "#4dedb4" : "#aaa";
                    
                    if(obj.physics) {
                        physConfigPanel.style.display = 'block';
                        document.getElementById('p_spring_k_val').value = obj.springK ?? 0.05;
                        document.getElementById('p_spring_d_val').value = obj.springD ?? 0.95;
                        document.getElementById('p_phys_profile').value = obj.physicsProfile || 'normal';
                        document.getElementById('p_auto_animate').checked = obj.autoAnimate || false;
                        document.getElementById('auto_var_name').innerText = `$auto_${objIndex}`;
                        document.getElementById('p_auto_str').value = obj.autoStr ?? 0.1;
                        document.getElementById('p_auto_source').value = getAutoSourceMode(obj);
                        document.getElementById('p_auto_amp_x').value = obj.autoAmpX ?? 1;
                        document.getElementById('p_auto_amp_y').value = obj.autoAmpY ?? 1;
                        document.getElementById('p_auto_seed_x').value = obj.autoSeedX ?? 0.3187;
                        document.getElementById('p_auto_seed_y').value = obj.autoSeedY ?? 0.6123;
                        document.getElementById('p_auto_func_x').value = obj.autoFuncX || (obj.type === 'joystick' ? 'sin(TAU * t)' : 'sin01(t)');
                        document.getElementById('p_auto_func_y').value = obj.autoFuncY || 'cos(TAU * t)';
                        document.getElementById('p_auto_speed').value = obj.autoSpeed ?? 0.015;
                        document.getElementById('p_auto_response').value = obj.autoResponse ?? 0.22;
                        document.getElementById('p_auto_bounce').value = obj.autoBounce ?? 0.25;
                        document.getElementById('p_gravity').value = obj.gravity ?? 0;
                        document.getElementById('p_chaos_rate').value = obj.chaosRate || 96;
                        if(obj.type === 'joystick') {
                            rowPhysProfile.style.display = 'flex';
                            physProfileHint.style.display = (obj.physicsProfile === 'breast') ? 'block' : 'none';
                        }
                        if(obj.type.includes('slider')) {
                            lblAutoAmpX.innerText = '自动范围:';
                            rowAutoAmpY.style.display = 'none';
                            lblAutoSeedX.innerText = '轨迹种子:';
                            rowAutoSeedY.style.display = 'none';
                        }
                        applyAutoEditorState(obj);
                    } else {
                        physConfigPanel.style.display = 'none';
                        document.getElementById('auto_var_name').innerText = '';
                    }
                }
            }
            renderVarInputs(obj);
        }
    }

    window.applyTextColor = (val) => {
        let o = selectedObj();
        if(!o) return;
        let ta = document.getElementById('p_text_content');
        let start = ta.selectionStart;
        let end = ta.selectionEnd;
        if (start === end) {
            o.fontColor = val;
        } else {
            if(!o.colorOverrides) o.colorOverrides = {};
            for(let i = start; i < end; i++) o.colorOverrides[i] = val;
        }
        renderAll();
    };

    window.updateSeqVar = () => {
        let o = selectedObj();
        if(o) {
            const next = sanitizeIniVarToken(document.getElementById('p_seq_var').value, '$State');
            document.getElementById('p_seq_var').value = next;
            o.seqVar = next;
        }
    };

    window.updateFollowCursor = () => {
        let o = selectedObj();
        if(!o || (o.type !== 'static' && o.type !== 'sequence')) return;
        o.followCursor = document.getElementById('p_follow_cursor').checked === true;
        document.getElementById('row_follow_offset').style.display = o.followCursor ? 'flex' : 'none';
        renderAll();
    };

    window.updateFollowOffset = () => {
        let o = selectedObj();
        if(!o || (o.type !== 'static' && o.type !== 'sequence')) return;
        const offsets = getFollowCursorOffsets({
            followOffsetX: parseFloat(document.getElementById('p_follow_offset_x').value),
            followOffsetY: parseFloat(document.getElementById('p_follow_offset_y').value)
        });
        o.followOffsetX = offsets.x;
        o.followOffsetY = offsets.y;
        document.getElementById('p_follow_offset_x').value = offsets.x;
        document.getElementById('p_follow_offset_y').value = offsets.y;
        renderAll();
    };

    window.updateZoneDragEnabled = () => {
        let o = selectedObj();
        if(!componentSupportsZoneDrag(o)) return;
        o.zoneDragEnabled = document.getElementById('p_zone_drag_enabled').checked === true;
        const rowDisplay = o.zoneDragEnabled ? 'flex' : 'none';
        document.getElementById('row_zone_drag_var').style.display = rowDisplay;
        document.getElementById('row_zone_drag_id').style.display = rowDisplay;
        document.getElementById('zone_drag_hint').style.display = o.zoneDragEnabled ? 'block' : 'none';
        renderAll();
    };

    window.updateZoneDragVar = () => {
        let o = selectedObj();
        if(!componentSupportsZoneDrag(o)) return;
        const next = getZoneDragVarName({ zoneDragVar: document.getElementById('p_zone_drag_var').value });
        document.getElementById('p_zone_drag_var').value = next;
        o.zoneDragVar = next;
    };

    window.updateZoneDragZoneId = () => {
        let o = selectedObj();
        if(!componentSupportsZoneDrag(o)) return;
        const next = getZoneDragZoneId({ zoneDragZoneId: document.getElementById('p_zone_drag_id').value });
        document.getElementById('p_zone_drag_id').value = next;
        o.zoneDragZoneId = next;
    };

    function escapeHtml(text) {
        return String(text ?? '')
            .replace(/&/g, '&amp;')
            .replace(/</g, '&lt;')
            .replace(/>/g, '&gt;')
            .replace(/"/g, '&quot;')
            .replace(/'/g, '&#39;');
    }

    function renderSeqList(obj) {
        const cont = document.getElementById('seq_list_container');
        cont.innerHTML = '';
        obj.frames.forEach((f, i) => {
            const div = document.createElement('div');
            div.className = 'seq-item';
            const thumbClass = f.preview ? 'seq-thumb' : 'seq-thumb empty';
            const thumbStyle = f.preview ? ` style="background-image:url(${f.preview})"` : '';
            div.innerHTML = `
                <div class="${thumbClass}"${thumbStyle}>${f.preview ? '' : '无'}</div>
                <span style="font-size:0.8em;color:var(--muted-color);">值:</span>
                <input type="number" class="seq-val-input" value="${f.val}" onchange="UIB.updateFrameVal(${i}, this.value)">
                <input type="text" class="res-path" value="${escapeHtml(f.path || '')}" readonly>
                <input type="file" id="u_seq_${i}" style="display:none" onchange="UIB.uploadSeqFrame(${i}, this)">
                <button class="res-upload" onclick="document.getElementById('u_seq_${i}').click()">上传</button>
                <button class="seq-del-btn" onclick="UIB.delFrame(${i})">删</button>
            `;
            cont.appendChild(div);
        });
    }

    window.addFrame = () => {
        let o = selectedObj();
        if(o) {
            o.frames.push({val: o.frames.length, path: '', preview: null});
            renderSeqList(o);
            refreshResourceWindow(o);
        }
    };
    window.delFrame = (i) => {
        let o = selectedObj();
        if(o) {
            o.frames.splice(i, 1);
            renderSeqList(o);
            refreshResourceWindow(o);
            renderAll();
        }
    };
    window.clearFrames = async () => {
        let o = selectedObj();
        if(o && await confirm("确认要清空所有序列帧吗？")) {
            o.frames = [];
            renderSeqList(o);
            refreshResourceWindow(o);
            renderAll();
        }
    };
    window.updateFrameVal = (i, val) => {
        let o = selectedObj();
        if(o) {
            o.frames[i].val = parseInt(val);
            refreshResourceWindow(o);
        }
    };
    function isSupportedTextureFile(file) {
        if(!file) return false;
        return String(file.type || '').startsWith('image/') || /\.(?:dds|tga)$/i.test(String(file.name || ''));
    }

    function handleResourceDragOver(event) {
        event.preventDefault();
        if(event.dataTransfer) event.dataTransfer.dropEffect = 'copy';
        event.currentTarget.classList.add('drag-over');
    }

    function handleResourceDragLeave(event) {
        if(event.relatedTarget && event.currentTarget.contains(event.relatedTarget)) return;
        event.currentTarget.classList.remove('drag-over');
    }

    function getDroppedTextureFile(event) {
        event.preventDefault();
        event.currentTarget.classList.remove('drag-over');
        const files = Array.from((event.dataTransfer && event.dataTransfer.files) || []);
        const file = files.find(isSupportedTextureFile);
        if(!file) alert('请拖入图片或 DDS/TGA 纹理文件。');
        return file || null;
    }

    async function assignSequenceFrameFile(obj, index, file) {
        if(!obj || !obj.frames || !obj.frames[index] || !isSupportedTextureFile(file)) return false;
        const dataUrl = await readFileAsDataURL(file);
        const frame = obj.frames[index];
        frame.path = normalizeAssetPath(file.name);
        frame.dataUrl = dataUrl;
        frame.preview = dataUrl;
        markHistoryDirty();
        renderSeqList(obj);
        refreshResourceWindow(obj);
        renderAll();
        return true;
    }

    async function dropSequenceFrame(event, index) {
        const file = getDroppedTextureFile(event);
        const obj = selectedObj();
        if(!file || !obj) return;
        try {
            await assignSequenceFrameFile(obj, index, file);
        } catch(err) {
            console.error(err);
            alert('读取序列帧失败。');
        }
    }

    window.uploadSeqFrame = async (i, input) => {
        const file = input.files[0];
        const obj = selectedObj();
        if(!file || !obj) return;
        try {
            await assignSequenceFrameFile(obj, i, file);
        } catch(err) {
            console.error(err);
            alert('读取序列帧失败。');
        }
        input.value = '';
    }
    window.batchUploadSeq = async (input) => {
        if(input.files && input.files.length > 0) {
            let o = selectedObj();
            if(!o) return;
            const files = Array.from(input.files).sort((a, b) => a.name.localeCompare(b.name, undefined, {numeric: true, sensitivity: 'base'}));
            const fileDataList = await Promise.all(files.map(async file => ({
                file,
                dataUrl: await readFileAsDataURL(file)
            })));

            fileDataList.forEach(({file, dataUrl}, idx) => {
                let val = idx;
                const match = file.name.match(/(\d+)/);
                if (match) val = parseInt(match[0]); else val = o.frames.length;
                o.frames.push({ val: val, path: normalizeAssetPath(file.name), preview: dataUrl, dataUrl: dataUrl });
            });
            input.value = '';
            renderSeqList(o);
            refreshResourceWindow(o);
            renderAll();
        }
    }

    function updateModeSelect(type) {
        modeSelect.innerHTML = '';
        if (type === 'toggle') {
            modeSelect.add(new Option("开关模式 (0 -> 1)", "1"));
            modeSelect.add(new Option("多档模式 (0 -> N)", TOGGLE_MULTI_MODE));
            return;
        }
        if (type.includes('slider')) {
            modeSelect.add(new Option("单值模式 (0 -> 1)", "1"));
            modeSelect.add(new Option("双向模式 (1 -> 0 -> 1)", "2"));
            modeSelect.add(new Option("多档模式 (0,1,2,...)", "3"));
        } else {
            modeSelect.add(new Option("XY 模式 (-1 ~ 1)", "2"));
            modeSelect.add(new Option("方向模式 (相邻方向混合)", "4"));
        }
    }
    window.changeMode = () => {
        let o = selectedObj();
        if(!o) return;
        o.paramMode = modeSelect.value;
        if (o.type === 'toggle') {
            normalizeToggleState(o);
            updatePropPanel();
            renderAll();
            return;
        }
        let ts = nextUniqueToken();
        if (o.type.includes('slider')) {
            o.sliderSubdivisions = 1;
            if (o.paramMode === '1') { 
                o.minVals = [0];
                o.vars = [`$Param_${ts}`]; 
                o.maxVals = [1];
                o.defVals = [0];
                o.depTargets = [[]];
                o.gridSteps = 3;
                setGridTargetEditorOpen(o, false);
            } else if (o.paramMode === '2') {
                o.minVals = [0, 0];
                o.vars = [`$Left_${ts}`, `$Right_${ts}`]; 
                o.maxVals = [1, 1];
                o.defVals = [0.5, 0.5];
                o.depTargets = [[], []];
                o.gridSteps = 3;
                setGridTargetEditorOpen(o, false);
            } else if (o.paramMode === '3') {
                o.gridSteps = 3;
                o.gridValueStart = 0;
                o.gridValueStep = 1;
                o.depTargets = [[]];
                normalizeSliderGridState(o, { createDefaultVar: true, resetGridTargets: true, resetMainVars: true });
                setGridTargetEditorOpen(o, false);
            }
        } else {
            if (o.paramMode === '2') { 
                o.vars = [`$JoyX_${ts}`, `$JoyY_${ts}`]; 
                o.maxVals = [1, 1];
                o.defVals = [o.joystickDefaultX || 0, o.joystickDefaultY || 0];
                o.depTargets = [[], []];
            } else { 
                o.joystickDirectionCount = o.joystickDirectionCount || 4;
                o.joystickSubdivisions = o.joystickSubdivisions || 1;
                o.joystickAngleOffset = normalizeAngle(o.joystickAngleOffset || 0);
                o.joystickDefaultX = 0;
                o.joystickDefaultY = 0;
                o.defVals = [];
                o.depTargets = [];
                o.maxVals = [];
                o.vars = [];
                ensureJoystickDirectionState(o, {resetNames: true});
            }
        }
        updatePropPanel(); 
        renderAll();
    };

    window.updateSwitchGroup = () => {
        let o = selectedObj();
        o.switchGroup = parseInt(document.getElementById('p_switch_group').value) || 0;
    };

    window.updateToggleSteps = () => {
        let o = selectedObj();
        if(!o || o.type !== 'toggle') return;
        o.toggleSteps = parseInt(document.getElementById('p_toggle_steps').value, 10);
        normalizeToggleState(o);
        document.getElementById('p_toggle_steps').value = o.toggleSteps;
        document.getElementById('p_initial_num').max = o.toggleSteps;
        document.getElementById('p_initial_num').value = o.initialValue;
        renderAll();
    };
    
    window.updateGridSteps = () => {
        let o = selectedObj();
        if(o && o.type.includes('slider')) {
            let newSteps = clampGridStepCount(document.getElementById('p_grid_steps').value, 3);
            o.gridSteps = newSteps;
            document.getElementById('p_grid_steps').value = newSteps;
            if(isSliderGridMode(o)) {
                normalizeSliderGridState(o);
                renderVarInputs(o);
            }
            renderAll();
        }
    };

    window.updateSliderSubdivisions = () => {
        const o = selectedObj();
        if(!o || !o.type || !o.type.includes('slider') || (o.paramMode !== '1' && o.paramMode !== '2')) return;
        const previousSubdivisions = getSliderSubdivisionConfig(o).subdivisions;
        const nextSubdivisions = clamp(parseInt(document.getElementById('p_slider_subdiv').value, 10) || 1, 1, 8);
        document.getElementById('p_slider_subdiv').value = nextSubdivisions;
        o.sliderSubdivisions = nextSubdivisions;
        ensureSliderSubdivisionState(o, { previousSubdivisions });
        clearPreviewSimulationCaches();
        updatePropPanel();
        renderAll();
    };

    window.updateJoystickDirectionCount = () => {
        let o = selectedObj();
        if(!o || o.type !== 'joystick' || o.paramMode !== '4') return;
        const nextCount = clamp(parseInt(document.getElementById('p_joy_dir_count').value, 10) || 4, 3, 32);
        document.getElementById('p_joy_dir_count').value = nextCount;
        o.joystickDirectionCount = nextCount;
        ensureJoystickDirectionState(o);
        renderVarInputs(o);
        renderAll();
    };

    window.updateJoystickSubdivisions = () => {
        let o = selectedObj();
        if(!o || o.type !== 'joystick' || o.paramMode !== '4') return;
        const nextSubdiv = clamp(parseInt(document.getElementById('p_joy_subdiv').value, 10) || 1, 1, 8);
        document.getElementById('p_joy_subdiv').value = nextSubdiv;
        o.joystickSubdivisions = nextSubdiv;
        ensureJoystickDirectionState(o);
        renderVarInputs(o);
        renderAll();
    };

    window.updateJoystickAngleOffset = () => {
        let o = selectedObj();
        if(!o || o.type !== 'joystick' || o.paramMode !== '4') return;
        const nextOffset = normalizeAngle(parseFloat(document.getElementById('p_joy_angle_offset').value) || 0);
        document.getElementById('p_joy_angle_offset').value = nextOffset;
        o.joystickAngleOffset = nextOffset;
        ensureJoystickDirectionState(o);
        renderVarInputs(o);
        renderAll();
    };

    window.updateJoystickDefaultX = () => {
        let o = selectedObj();
        if(!o || o.type !== 'joystick') return;
        const v = clamp(parseFloat(document.getElementById('p_joy_default_x').value) || 0, -1, 1);
        document.getElementById('p_joy_default_x').value = v.toFixed(2);
        o.joystickDefaultX = v;
        if(!o.defVals) o.defVals = [];
        o.defVals[0] = v;
        const runtime = previewInteractiveRuntime.get(o.id);
        if(runtime) { runtime.valueX = v; }
        clearPreviewSimulationCaches();
        renderAll();
    };

    window.updateJoystickDefaultY = () => {
        let o = selectedObj();
        if(!o || o.type !== 'joystick') return;
        const v = clamp(parseFloat(document.getElementById('p_joy_default_y').value) || 0, -1, 1);
        document.getElementById('p_joy_default_y').value = v.toFixed(2);
        o.joystickDefaultY = v;
        if(!o.defVals) o.defVals = [];
        o.defVals[1] = v;
        const runtime = previewInteractiveRuntime.get(o.id);
        if(runtime) { runtime.valueY = v; }
        clearPreviewSimulationCaches();
        renderAll();
    };

    // ========== 嵌套联动 UI 函数 ==========
    
    /** 获取可被选为联动目标的组件列表（排除自己） */
    function getLinkableComponents() {
        const obj = selectedObj();
        if(!obj) return [];
        const isDirMode = obj.type === 'joystick' && obj.paramMode === '4';
        if(isDirMode) {
            return components.filter(c => c && c.id !== obj.id &&
                (c.type === 'joystick' || c.type.includes('slider')));
        }
        return components.filter(c => c && c.id !== obj.id && 
            (c.type.includes('slider') || c.type === 'joystick'));
    }
    // ========== 积蓄条 UI 函数 ==========
    function getAccumLinkableComponents() {
        const obj = selectedObj();
        if(!obj) return [];
        return components.filter(c => c && c.id !== obj.id &&
            (c.type.includes('slider') || c.type === 'joystick' || c.type === 'toggle'));
    }

    window.addAccumBinding = () => {
        const o = selectedObj();
        if(!o || o.type !== 'accum') return;
        if(!Array.isArray(o.accumBindings)) o.accumBindings = [];
        o.accumBindings.push({ targetId: '', kind: 'auto' });
        renderAccumBindings();
    };

    window.updateAccumBinding = (idx, val) => {
        const o = selectedObj();
        if(!o || !Array.isArray(o.accumBindings) || !o.accumBindings[idx]) return;
        o.accumBindings[idx].targetId = val;
        renderAccumBindings();
        renderAll();
    };

    window.removeAccumBinding = (idx) => {
        const o = selectedObj();
        if(!o || !Array.isArray(o.accumBindings)) return;
        o.accumBindings.splice(idx, 1);
        renderAccumBindings();
        renderAll();
    };

    window.addAccumTrigger = () => {
        const o = selectedObj();
        if(!o || o.type !== 'accum') return;
        if(!Array.isArray(o.accumTriggers)) o.accumTriggers = [];
        o.accumTriggers.push({ var: '', value: 0 });
        renderAccumTriggers();
    };

    window.updateAccumTriggerVar = (idx, val) => {
        const o = selectedObj();
        if(!o || !Array.isArray(o.accumTriggers) || !o.accumTriggers[idx]) return;
        const cleaned = sanitizeIniVarToken(val, '');
        o.accumTriggers[idx].var = cleaned;
        const input = document.querySelectorAll('#accum_triggers_list input[type=text]')[idx];
        if(input) input.value = cleaned;
        renderAll();
    };

    window.updateAccumTriggerValue = (idx, val) => {
        const o = selectedObj();
        if(!o || !Array.isArray(o.accumTriggers) || !o.accumTriggers[idx]) return;
        o.accumTriggers[idx].value = Number.isFinite(Number(val)) ? Number(val) : 0;
        renderAll();
    };

    window.removeAccumTrigger = (idx) => {
        const o = selectedObj();
        if(!o || !Array.isArray(o.accumTriggers)) return;
        o.accumTriggers.splice(idx, 1);
        renderAccumTriggers();
        renderAll();
    };

    window.updateAccumProps = () => {
        const o = selectedObj();
        if(!o || o.type !== 'accum') return;
        const dir = document.getElementById('p_accum_direction').value === 'v' ? 'v' : 'h';
        const parsedThreshold = parseFloat(document.getElementById('p_accum_threshold').value);
        o.accumDirection = dir;
        o.accumThreshold = Number.isFinite(parsedThreshold) && parsedThreshold > 0 ? parsedThreshold : 5;
        document.getElementById('p_accum_threshold').value = o.accumThreshold;
        renderAll();
    };

    function renderAccumBindings() {
        const o = selectedObj();
        const container = document.getElementById('accum_bindings_list');
        if(!container || !o) return;
        container.innerHTML = '';
        const bindings = Array.isArray(o.accumBindings) ? o.accumBindings : [];
        if(bindings.length === 0) {
            container.innerHTML = '<div style="font-size:0.72em; color:var(--muted-color); padding:4px;">暂无绑定，添加后统计对应组件的拖拽位移或点击。</div>';
            return;
        }
        const linkable = getAccumLinkableComponents();
        bindings.forEach((b, idx) => {
            const targetComp = components.find(c => c.id === b.targetId);
            const kindLabel = targetComp && targetComp.type === 'toggle' ? '点击计数' : (targetComp && targetComp.type === 'joystick' ? '双轴位移' : '拖拽位移');
            container.innerHTML += `
                <div style="display:flex; gap:4px; align-items:center; background:rgba(0,0,0,0.25); padding:4px 6px; border:1px solid rgba(255,90,90,0.3); border-radius:4px;">
                    <span style="font-size:0.7em; color:#ff8a96; white-space:nowrap;">绑定 ${idx + 1}:</span>
                    <select onchange="UIB.updateAccumBinding(${idx},this.value)" style="flex:1; font-size:0.78em; padding:2px; background:var(--input-bg); border:1px solid var(--border-color); color:var(--text-color); border-radius:6px;">
                        <option value="">-- 选择组件 --</option>
                        ${linkable.map(c => `<option value="${c.id}" ${c.id === b.targetId ? 'selected' : ''}>${getComponentDisplayName(c)}</option>`).join('')}
                    </select>
                    ${targetComp ? `<span style="font-size:0.65em; color:#a8ecff; white-space:nowrap;">${kindLabel}</span>` : ''}
                    <button onclick="UIB.removeAccumBinding(${idx})" style="font-size:0.65em; padding:1px 6px; background:var(--uib-btn-danger); border:1px solid #7e3644; color:#ff5f74; cursor:pointer; border-radius:3px; width:auto;">删</button>
                </div>
            `;
        });
    }

    function renderAccumTriggers() {
        const o = selectedObj();
        const container = document.getElementById('accum_triggers_list');
        if(!container || !o) return;
        container.innerHTML = '';
        const triggers = Array.isArray(o.accumTriggers) ? o.accumTriggers : [];
        if(triggers.length === 0) {
            container.innerHTML = '<div style="font-size:0.72em; color:var(--muted-color); padding:4px;">未配置触发变量。</div>';
            return;
        }
        triggers.forEach((t, idx) => {
            container.innerHTML += `
                <div style="display:flex; gap:4px; align-items:center; background:rgba(0,0,0,0.25); padding:4px 6px; border:1px solid rgba(255,190,90,0.3); border-radius:4px;">
                    <span style="font-size:0.7em; color:var(--uib-warn-text); white-space:nowrap;">变量 ${idx + 1}:</span>
                    <input type="text" value="${escapeHtml(t.var || '')}" placeholder="$Var"
                        onchange="UIB.updateAccumTriggerVar(${idx},this.value)"
                        style="flex:1; font-size:0.75em; padding:2px; color:#ffcf7d; border:1px solid rgba(255,181,77,0.45); background:var(--input-bg); border-radius:4px;">
                    <span style="font-size:0.7em; color:var(--uib-warn-text); white-space:nowrap;">=</span>
                    <input type="number" step="0.01" value="${Number.isFinite(Number(t.value)) ? Number(t.value) : 0}"
                        onchange="UIB.updateAccumTriggerValue(${idx},this.value)"
                        style="width:84px; font-size:0.75em; padding:2px; background:var(--input-bg); border:1px solid var(--border-color); color:var(--text-color); border-radius:4px;">
                    <button onclick="UIB.removeAccumTrigger(${idx})" style="font-size:0.65em; padding:1px 6px; background:var(--uib-btn-danger); border:1px solid #7e3644; color:#ff5f74; cursor:pointer; border-radius:3px; width:auto;">删</button>
                </div>
            `;
        });
    }

   function renderLinkedSlaveActionsEditor(linkIdx, phase, actions) {
        const safePhase = phase === 'leave' ? 'leave' : 'enter';
        const title = safePhase === 'leave' ? '离开区间触发' : '进入区间触发';
        const hint = safePhase === 'leave' ? '离开该区间时执行一次' : '进入该区间时执行一次';
        const rows = actions.map((action, actionIdx) => `
            <div style="display:flex; gap:4px; align-items:center; margin-top:4px;">
                <input type="text" value="${action.var || ''}" placeholder="$TargetVar"
                    onchange="UIB.updateLinkedSlaveActionVar(${linkIdx},'${safePhase}',${actionIdx},this.value)"
                    style="flex:1; font-size:0.75em; padding:2px; color:#ffcf7d; border:1px solid rgba(255,181,77,0.45); background:var(--input-bg); border-radius:4px;">
                <input type="number" step="0.01" value="${Number.isFinite(Number(action.value)) ? Number(action.value) : 0}"
                    onchange="UIB.updateLinkedSlaveActionValue(${linkIdx},'${safePhase}',${actionIdx},this.value)"
                    style="width:92px; font-size:0.75em; padding:2px; background:var(--input-bg); border:1px solid var(--border-color); color:var(--text-color); border-radius:4px;">
                <button onclick="UIB.removeLinkedSlaveAction(${linkIdx},'${safePhase}',${actionIdx})"
                    style="font-size:0.7em; padding:2px 6px; background:var(--uib-btn-danger); border:1px solid #7e3644; color:#ff5f74; cursor:pointer; border-radius:3px; width:auto;">删</button>
            </div>
        `).join('');
        return `
            <div style="margin-top:6px; padding:5px; background:rgba(255,181,77,0.06); border:1px solid rgba(255,181,77,0.22); border-radius:4px;">
                <div style="display:flex; justify-content:space-between; align-items:center; gap:6px;">
                    <div>
                        <div style="font-size:0.76em; color:#ffd98a; font-weight:bold;">${title}</div>
                        <div style="font-size:0.65em; color:var(--muted-color);">${hint}</div>
                    </div>
                    <button onclick="UIB.addLinkedSlaveAction(${linkIdx},'${safePhase}')"
                        style="font-size:0.68em; padding:2px 8px; background:var(--uib-btn-ok); border:1px solid #2f8a62; color:#7df0c0; cursor:pointer; border-radius:3px; width:auto;">+ 参数</button>
                </div>
                ${rows || '<div style="font-size:0.68em; color:var(--muted-color); margin-top:4px;">未配置动作</div>'}
            </div>
        `;
    }

    /**
     * 渲染从属联动面板: 列出当前选中组件的所有联动, 显示源/目标/映射配置
     */
    function renderLinkedSlaves() {
        const obj = selectedObj();
        const container = document.getElementById('linked_slaves_list');
        if(!container || !obj) return;
        container.innerHTML = '';
        const slaves = Array.isArray(obj.linkedSlaves) ? obj.linkedSlaves : [];
        if(slaves.length === 0) {
            container.innerHTML = '<div style="font-size:0.75em; color:var(--muted-color); padding:4px;">暂无联动目标，点击下方按钮添加。</div>';
            return;
        }
        const linkable = getLinkableComponents();
        // 一次性计算所有联动值，避免重复计算
        let allLinkedValues;
        try { allLinkedValues = computeAllLinkedValues(); } catch(e) { console.error('computeAllLinkedValues error:', e); allLinkedValues = new Map(); }
        slaves.forEach((slave, idx) => {
            try {  // 捕获单个联动项渲染错误，防止整个面板崩溃
            const targetComp = components.find(c => c.id === slave.targetId);
            const isJoystick = obj.type === 'joystick';
            const targetIsJoystick = targetComp && targetComp.type === 'joystick';
            const srcMin = slave.srcMin ?? 0;
            const srcMax = slave.srcMax ?? 0.5;
            const overflow = slave.overflow || 'reset';
            const splitSide = slave.splitSide || 'both';
            const joyAxis = (slave.joyAxis !== undefined && slave.joyAxis !== null && slave.joyAxis !== '') ? slave.joyAxis : 'both';
            const joyTargetAxis = (slave.joyTargetAxis !== undefined && slave.joyTargetAxis !== null && slave.joyTargetAxis !== '') ? slave.joyTargetAxis : 'x';
            const enterActions = normalizeLinkedSlaveActionList(slave.enterActions);
            const leaveActions = normalizeLinkedSlaveActionList(slave.leaveActions);
            
            let html = `<div style="background:rgba(0,0,0,0.3); padding:6px; border:1px solid rgba(255,165,0,0.3); border-radius:4px; margin-bottom:4px;">`;
            html += `<div style="display:flex; justify-content:space-between; align-items:center; margin-bottom:4px;">
                <span style="font-size:0.8em; color:#ffab3d; font-weight:bold;">联动 ${idx + 1}</span>
                <button onclick="UIB.removeLinkedSlave(${idx})" style="font-size:0.65em; padding:1px 6px; background:var(--uib-btn-danger); border:1px solid #7e3644; color:#ff5f74; cursor:pointer; border-radius:2px; width:auto;">删除</button>
            </div>`;
            
            // 目标组件选择
            html += `<div class="input-row" style="margin-bottom:3px;">
                <label style="font-size:0.75em; color:#ffab3d; flex:0 0 5em;">目标组件:</label>
                <select onchange="UIB.updateLinkedSlave(${idx},'targetId',this.value)" style="flex:1; font-size:0.8em; padding:2px; background:var(--input-bg); border:1px solid var(--border-color); color:var(--text-color); border-radius:6px;">
                    <option value="">-- 选择组件 --</option>
                    ${linkable.map(c => `<option value="${c.id}" ${c.id === slave.targetId ? 'selected' : ''}>${getComponentDisplayName(c)}</option>`).join('')}
                </select>
            </div>`;
            
            // 摇杆联动面板
            const isDirMode = obj.type === 'joystick' && obj.paramMode === '4';
            const regionMode = getLinkedSlaveEffectiveRegionMode(slave, obj, targetComp);
            const sourceToJoystick = isJoystick && targetIsJoystick;

            if(sourceToJoystick) {
                const pts = getLinkedSlaveRegionPoints(slave);
                html += `<div style="font-size:0.68em; color:#a8ecff; margin-bottom:5px; background:rgba(63,217,255,0.08); padding:4px 6px; border:1px solid rgba(63,217,255,0.2); border-radius:4px;">
                    摇杆联动摇杆固定使用 4 点区域映射。源摇杆进入该区域后，位置会直接映射到目标摇杆。
                </div>`;
                html += `<div style="background:rgba(0,0,0,0.2); padding:6px; border:1px solid rgba(255,165,0,0.2); border-radius:4px; margin-bottom:3px;">
                    <div style="font-size:0.75em; color:#ffab3d; font-weight:bold; margin-bottom:4px;">四边形区域 (4 点)</div>
                    <div style="display:grid; grid-template-columns:1fr 1fr; gap:4px 6px;">`;
                const ptLabels = ['点1 (左上)', '点2 (右上)', '点3 (右下)', '点4 (左下)'];
                const ptColors = ['#ff9daa', '#7df0c0', '#9d8fff', '#ffe08a'];
                for(let pi = 0; pi < 4; pi++) {
                    const p = pts[pi] || { x: 0, y: 0 };
                    html += `<div style="border:1px solid ${ptColors[pi]}33; padding:3px; border-radius:3px;">
                        <label style="font-size:0.6em; color:${ptColors[pi]};">${ptLabels[pi]}</label>
                        <div style="display:flex; gap:3px; align-items:center;">
                            <span style="font-size:0.55em; color:var(--muted-color);">X</span>
                            <input type="number" step="0.01" min="-1" max="1" value="${p.x.toFixed(2)}"
                                onchange="UIB.updateLinkedSlaveRegionPoint(${idx},${pi},'x',this.value)"
                                style="flex:1; font-size:0.7em; padding:1px; background:var(--input-bg); border:1px solid var(--border-color); color:#ff8a96; border-radius:3px;">
                            <span style="font-size:0.55em; color:var(--muted-color);">Y</span>
                            <input type="number" step="0.01" min="-1" max="1" value="${p.y.toFixed(2)}"
                                onchange="UIB.updateLinkedSlaveRegionPoint(${idx},${pi},'y',this.value)"
                                style="flex:1; font-size:0.7em; padding:1px; background:var(--input-bg); border:1px solid var(--border-color); color:#a58bff; border-radius:3px;">
                        </div>
                    </div>`;
                }
                const center = getLinkedSlaveRegionCenter(pts);
                html += `</div>
                    <div style="font-size:0.62em; color:var(--muted-color); margin-top:4px;">
                        区域中心 (碰撞桩): X=${center.x.toFixed(3)} Y=${center.y.toFixed(3)}
                    </div>
                </div>`;
                html += `<div class="input-row" style="margin-bottom:3px;">
                    <label style="font-size:0.75em; color:#ffab3d; flex:0 0 5em;">超出范围:</label>
                    <select onchange="UIB.updateLinkedSlave(${idx},'overflow',this.value)" style="flex:1; font-size:0.8em; padding:2px; background:var(--input-bg); border:1px solid var(--border-color); color:var(--text-color); border-radius:6px;">
                        <option value="reset" ${overflow === 'reset' ? 'selected' : ''}>归零（带回弹）</option>
                        <option value="keep_max" ${overflow === 'keep_max' ? 'selected' : ''}>保持最大值</option>
                    </select>
                </div>`;
            } else {
                if(isJoystick) {
                    if(isDirMode) {
                        const dirOpts = [];
                        for(let d = 0; d < getJoystickConfig(obj).directionCount; d++) {
                            const angle = getJoystickDirectionAngle(obj, d);
                            const label = formatAngleLabel(angle);
                            dirOpts.push(`<option value="${d}" ${String(joyAxis) === String(d) ? 'selected' : ''}>方向 ${d+1} · ${label}</option>`);
                        }
                        html += `<div class="input-row" style="margin-bottom:3px;">
                            <label style="font-size:0.75em; color:#3fd9ff; flex:0 0 5em;">源方向:</label>
                            <select onchange="UIB.updateLinkedSlave(${idx},'joyAxis',this.value)" style="flex:1; font-size:0.8em; padding:2px; background:var(--input-bg); border:1px solid var(--border-color); color:var(--text-color); border-radius:6px;">
                                ${dirOpts.join('')}
                            </select>
                        </div>`;
                        if(targetComp && targetComp.paramMode === '2') {
                            const joyAxis2 = slave.joyAxis2;
                            const dirOpts2 = [`<option value="" ${!joyAxis2 ? 'selected' : ''}>未启用</option>`];
                            for(let d = 0; d < getJoystickConfig(obj).directionCount; d++) {
                                const angle = getJoystickDirectionAngle(obj, d);
                                const label = formatAngleLabel(angle);
                                dirOpts2.push(`<option value="${d}" ${String(joyAxis2) === String(d) ? 'selected' : ''}>方向 ${d+1} · ${label}</option>`);
                            }
                            html += `<div class="input-row" style="margin-bottom:3px;">
                                <label style="font-size:0.75em; color:#ffd98a; flex:0 0 5em;">右半方向:</label>
                                <select onchange="UIB.updateLinkedSlave(${idx},'joyAxis2',this.value)" style="flex:1; font-size:0.8em; padding:2px; background:var(--input-bg); border:1px solid rgba(255,207,125,0.45); color:#ffd98a; border-radius:6px;">
                                    ${dirOpts2.join('')}
                                </select>
                            </div>`;
                        }
                    } else {
                        html += `<div class="input-row" style="margin-bottom:3px;">
                            <label style="font-size:0.75em; color:#ffab3d; flex:0 0 5em;">源轴:</label>
                            <select onchange="UIB.updateLinkedSlave(${idx},'joyAxis',this.value)" style="flex:1; font-size:0.8em; padding:2px; background:var(--input-bg); border:1px solid var(--border-color); color:var(--text-color); border-radius:6px;">
                                <option value="both" ${joyAxis === 'both' ? 'selected' : ''}>两轴(取最大值)</option>
                                <option value="x" ${joyAxis === 'x' ? 'selected' : ''}>X 轴</option>
                                <option value="y" ${joyAxis === 'y' ? 'selected' : ''}>Y 轴</option>
                            </select>
                        </div>`;
                        if(targetComp && targetComp.paramMode === '2') {
                            const joyAxis2 = slave.joyAxis2;
                            html += `<div class="input-row" style="margin-bottom:3px;">
                                <label style="font-size:0.75em; color:#ffd98a; flex:0 0 5em;">右半轴:</label>
                                <select onchange="UIB.updateLinkedSlave(${idx},'joyAxis2',this.value)" style="flex:1; font-size:0.8em; padding:2px; background:var(--input-bg); border:1px solid rgba(255,207,125,0.45); color:#ffd98a; border-radius:6px;">
                                    <option value="" ${!joyAxis2 ? 'selected' : ''}>未启用</option>
                                    <option value="x" ${joyAxis2 === 'x' ? 'selected' : ''}>X 轴</option>
                                    <option value="y" ${joyAxis2 === 'y' ? 'selected' : ''}>Y 轴</option>
                                </select>
                            </div>`;
                        }
                    }
                }

                if(!isJoystick && targetIsJoystick && targetComp.paramMode !== '4') {
                    html += `<div class="input-row" style="margin-bottom:3px;">
                        <label style="font-size:0.75em; color:#ffab3d; flex:0 0 5em;">目标轴:</label>
                        <select onchange="UIB.updateLinkedSlave(${idx},'joyTargetAxis',this.value)" style="flex:1; font-size:0.8em; padding:2px; background:var(--input-bg); border:1px solid var(--border-color); color:var(--text-color); border-radius:6px;">
                            <option value="x" ${joyTargetAxis === 'x' ? 'selected' : ''}>X 轴</option>
                            <option value="y" ${joyTargetAxis === 'y' ? 'selected' : ''}>Y 轴</option>
                            <option value="both" ${joyTargetAxis === 'both' ? 'selected' : ''}>双轴</option>
                        </select>
                    </div>`;
                } else if(!isJoystick && targetIsJoystick && targetComp.paramMode === '4') {
                    const tgtDirCount = getJoystickConfig(targetComp).directionCount;
                    const dirOpts = [];
                    for(let d = 0; d < tgtDirCount; d++) {
                        const angle = getJoystickDirectionAngle(targetComp, d);
                        const label = formatAngleLabel(angle);
                        dirOpts.push(`<option value="${d}" ${String(joyTargetAxis) === String(d) ? 'selected' : ''}>方向 ${d+1} · ${label}</option>`);
                    }
                    html += `<div class="input-row" style="margin-bottom:3px;">
                        <label style="font-size:0.75em; color:#ffab3d; flex:0 0 5em;">目标方向:</label>
                        <select onchange="UIB.updateLinkedSlave(${idx},'joyTargetAxis',this.value)" style="flex:1; font-size:0.8em; padding:2px; background:var(--input-bg); border:1px solid var(--border-color); color:var(--text-color); border-radius:6px;">
                            ${dirOpts.join('')}
                        </select>
                    </div>`;
                    if(obj.paramMode === '2') {
                        const joyTargetAxis2 = slave.joyTargetAxis2;
                        const dirOpts2 = [`<option value="" ${!joyTargetAxis2 ? 'selected' : ''}>未选择</option>`];
                        for(let d = 0; d < tgtDirCount; d++) {
                            const angle = getJoystickDirectionAngle(targetComp, d);
                            const label = formatAngleLabel(angle);
                            dirOpts2.push(`<option value="${d}" ${String(joyTargetAxis2) === String(d) ? 'selected' : ''}>方向 ${d+1} · ${label}</option>`);
                        }
                        html += `<div class="input-row" style="margin-bottom:3px;">
                            <label style="font-size:0.75em; color:#ffd98a; flex:0 0 5em;">目标方向 2:</label>
                            <select onchange="UIB.updateLinkedSlave(${idx},'joyTargetAxis2',this.value)" style="flex:1; font-size:0.8em; padding:2px; background:var(--input-bg); border:1px solid rgba(255,207,125,0.45); color:#ffd98a; border-radius:6px;">
                                ${dirOpts2.join('')}
                            </select>
                        </div>`;
                    }
                }

                html += `<div style="display:flex; gap:4px; margin-bottom:3px;">
                    <div style="flex:1;">
                        <label style="font-size:0.7em; color:var(--muted-color);">源区间最小</label>
                        <input type="number" step="0.01" min="0" max="1" value="${srcMin}" 
                            onchange="UIB.updateLinkedSlaveNum(${idx},'srcMin',this.value)" 
                            style="width:100%; font-size:0.8em; padding:2px; background:var(--input-bg); border:1px solid var(--border-color); color:var(--text-color); border-radius:6px;">
                    </div>
                    <div style="flex:1;">
                        <label style="font-size:0.7em; color:var(--muted-color);">源区间最大</label>
                        <input type="number" step="0.01" min="0" max="1" value="${srcMax}"
                            onchange="UIB.updateLinkedSlaveNum(${idx},'srcMax',this.value)"
                            style="width:100%; font-size:0.8em; padding:2px; background:var(--input-bg); border:1px solid var(--border-color); color:var(--text-color); border-radius:6px;">
                    </div>
                </div>`;

                if(targetComp && targetComp.paramMode === '2') {
                    html += `<div class="input-row" style="margin-bottom:3px;">
                        <label style="font-size:0.75em; color:#ffab3d; flex:0 0 5em;">映射方向:</label>
                        <select onchange="UIB.updateLinkedSlave(${idx},'splitSide',this.value)" style="flex:1; font-size:0.8em; padding:2px; background:var(--input-bg); border:1px solid var(--border-color); color:var(--text-color); border-radius:6px;">
                            <option value="both" ${splitSide === 'both' ? 'selected' : ''}>整体</option>
                            <option value="left" ${splitSide === 'left' ? 'selected' : ''}>左/下半 (0.5→0)</option>
                            <option value="right" ${splitSide === 'right' ? 'selected' : ''}>右/上半 (0.5→1)</option>
                        </select>
                    </div>`;
                }

                html += `<div class="input-row" style="margin-bottom:3px;">
                    <label style="font-size:0.75em; color:#ffab3d; flex:0 0 5em;">超出范围:</label>
                    <select onchange="UIB.updateLinkedSlave(${idx},'overflow',this.value)" style="flex:1; font-size:0.8em; padding:2px; background:var(--input-bg); border:1px solid var(--border-color); color:var(--text-color); border-radius:6px;">
                        <option value="reset" ${overflow === 'reset' ? 'selected' : ''}>归零（带回弹）</option>
                        <option value="keep_max" ${overflow === 'keep_max' ? 'selected' : ''}>保持最大值</option>
                    </select>
                </div>`;
            }
            
            // 碰撞桩配置（方向模式联动时 或 四边形区域联动时可用）
            const postEnabled = slave.postEnabled === true;
            const postRadius = getLinkedSlavePostRadius(slave);
            if(isJoystick && targetIsJoystick) {
                html += `<div style="margin-top:6px; padding:4px; background:rgba(64,224,208,0.08); border:1px solid rgba(64,224,208,0.35); border-radius:3px;">
                    <label style="font-size:0.8em; color:#4dedb4; font-weight:bold;">
                        <input type="checkbox" ${postEnabled ? 'checked' : ''} onchange="UIB.updateLinkedSlave(${idx},'postEnabled',this.checked)" style="margin-right:4px;">碰撞桩（中心激活）
                    </label>
                    <div id="post_config_${idx}" style="display:${postEnabled ? 'flex' : 'none'}; gap:4px; margin-top:4px;">
                        <div style="flex:1;">
                            <label style="font-size:0.65em; color:var(--muted-color);">检测半径（区域占比）</label>
                            <input type="number" step="0.01" min="0" max="1" value="${postRadius}"
                                onchange="UIB.updateLinkedSlaveNum(${idx},'postRadius',this.value)"
                                style="width:100%; font-size:0.75em; padding:1px; background:var(--input-bg); border:1px solid var(--border-color); color:var(--text-color); border-radius:4px;">
                        </div>
                    </div>
                </div>`;
            }
            
            // 预览当前映射值
            if(targetComp) {
                const mappedVal = allLinkedValues.get(slave.targetId);
                let displayVal = 'N/A';
                if(typeof mappedVal === 'number') {
                    displayVal = mappedVal.toFixed(3);
                } else if(mappedVal && typeof mappedVal === 'object') {
                    if(typeof mappedVal.x === 'number') {
                        displayVal = `X:${mappedVal.x.toFixed(3)} Y:${(mappedVal.y??0).toFixed(3)}`;
                    } else {
                        // 方向摇杆目标：显示各方向值
                        const dirParts = [];
                        const cfg = targetComp.paramMode === '4' ? getJoystickConfig(targetComp) : null;
                        const dirCount = cfg ? cfg.directionCount : 0;
                        for(let d = 0; d < dirCount; d++) {
                            if(typeof mappedVal[d] === 'number') {
                                dirParts.push(`D${d}:${mappedVal[d].toFixed(2)}`);
                            }
                        }
                        displayVal = dirParts.length > 0 ? dirParts.join(' ') : 'N/A';
                    }
                }
                html += `<div style="font-size:0.7em; color:#8fe8ff; margin-top:3px; padding:2px 4px; background:rgba(0,0,0,0.2); border-radius:2px;">映射值: ${displayVal}</div>`;
            }

            html += renderLinkedSlaveActionsEditor(idx, 'enter', enterActions);
            html += renderLinkedSlaveActionsEditor(idx, 'leave', leaveActions);
            
            html += `</div>`;
            container.innerHTML += html;
            } catch(e) { console.error('renderLinkedSlaves item error:', e, slave); }
        });
    }

    window.addLinkedSlave = () => {
        const obj = selectedObj();
        if(!obj) return;
        if(!Array.isArray(obj.linkedSlaves)) obj.linkedSlaves = [];
        const isJoystick = obj.type === 'joystick';
        const entry = {
            targetId: '',
            srcMin: 0,
            srcMax: 0.5,
            srcMinY: -1,
            srcMaxY: 1,
            regionPoints: isJoystick ? createDefaultLinkedSlaveRegionPoints() : createDefaultLinkedSlaveRegionPoints(),
            regionMode: isJoystick ? 'rect' : 'single',
            overflow: 'reset',
            splitSide: 'both',
            joyAxis: isJoystick ? 'both' : 'both',
            joyTargetAxis: 'both',
            postEnabled: false,
            postRadius: 0.25,
            enabled: true,
            enterActions: [],
            leaveActions: []
        };
        obj.linkedSlaves.push(entry);
        renderLinkedSlaves();
        markHistoryDirty();
    };

    window.updateGridValueRange = () => {
        const o = selectedObj();
        if(!isSliderGridMode(o)) return;
        const startInput = document.getElementById('p_grid_value_start');
        const stepInput = document.getElementById('p_grid_value_step');
        const nextStart = Number(startInput.value);
        const nextStep = Number(stepInput.value);
        o.gridValueStart = Number.isFinite(nextStart) ? nextStart : 0;
        o.gridValueStep = Number.isFinite(nextStep) && Math.abs(nextStep) > 1e-12 ? nextStep : 1;
        startInput.value = o.gridValueStart;
        stepInput.value = o.gridValueStep;
        clearPreviewSimulationCaches();
        renderVarInputs(o);
        renderAll();
    };

    window.removeLinkedSlave = (idx) => {
        const obj = selectedObj();
        if(!obj || !Array.isArray(obj.linkedSlaves)) return;
        obj.linkedSlaves.splice(idx, 1);
        renderLinkedSlaves();
        renderAll();
        markHistoryDirty();
    };

    window.updateLinkedSlave = (idx, field, value) => {
        const obj = selectedObj();
        if(!obj || !Array.isArray(obj.linkedSlaves) || !obj.linkedSlaves[idx]) return;
        obj.linkedSlaves[idx][field] = value;
        clearPreviewSimulationCaches();
        renderLinkedSlaves();
        renderAll();
        markHistoryDirty();
    };

    window.updateLinkedSlaveNum = (idx, field, value) => {
        const obj = selectedObj();
        if(!obj || !Array.isArray(obj.linkedSlaves) || !obj.linkedSlaves[idx]) return;
        obj.linkedSlaves[idx][field] = parseFloat(value) || 0;
        clearPreviewSimulationCaches();
        renderLinkedSlaves();
        renderAll();
        markHistoryDirty();
    };

    /** 更新四边形区域的单个点坐标 */
    window.updateLinkedSlaveRegionPoint = (idx, ptIdx, axis, value) => {
        const obj = selectedObj();
        if(!obj || !Array.isArray(obj.linkedSlaves) || !obj.linkedSlaves[idx]) return;
        const slave = obj.linkedSlaves[idx];
        if(!Array.isArray(slave.regionPoints)) slave.regionPoints = [];
        if(!slave.regionPoints[ptIdx]) slave.regionPoints[ptIdx] = {x:0, y:0};
        slave.regionPoints[ptIdx][axis] = parseFloat(value) || 0;
        // 同步更新旧的 srcMin/srcMax/srcMinY/srcMaxY (兼容)
        if(ptIdx === 0) { slave.srcMin = slave.regionPoints[0].x; slave.srcMaxY = slave.regionPoints[0].y; }
        if(ptIdx === 1) { slave.srcMax = slave.regionPoints[1].x; slave.srcMaxY = slave.regionPoints[1].y; }
        if(ptIdx === 2) { slave.srcMax = slave.regionPoints[2].x; slave.srcMinY = slave.regionPoints[2].y; }
        if(ptIdx === 3) { slave.srcMin = slave.regionPoints[3].x; slave.srcMinY = slave.regionPoints[3].y; }
        clearPreviewSimulationCaches();
        renderLinkedSlaves();
        renderAll();
        markHistoryDirty();
    };

    window.addLinkedSlaveAction = (idx, phase) => {
        const obj = selectedObj();
        if(!obj || !Array.isArray(obj.linkedSlaves) || !obj.linkedSlaves[idx]) return;
        const key = phase === 'leave' ? 'leaveActions' : 'enterActions';
        if(!Array.isArray(obj.linkedSlaves[idx][key])) obj.linkedSlaves[idx][key] = [];
        obj.linkedSlaves[idx][key].push({ var: '', value: 0 });
        clearPreviewSimulationCaches();
        renderLinkedSlaves();
        renderAll();
        markHistoryDirty();
    };

    window.removeLinkedSlaveAction = (idx, phase, actionIdx) => {
        const obj = selectedObj();
        if(!obj || !Array.isArray(obj.linkedSlaves) || !obj.linkedSlaves[idx]) return;
        const key = phase === 'leave' ? 'leaveActions' : 'enterActions';
        if(!Array.isArray(obj.linkedSlaves[idx][key]) || !obj.linkedSlaves[idx][key][actionIdx]) return;
        obj.linkedSlaves[idx][key].splice(actionIdx, 1);
        clearPreviewSimulationCaches();
        renderLinkedSlaves();
        renderAll();
        markHistoryDirty();
    };

    window.updateLinkedSlaveActionVar = (idx, phase, actionIdx, value) => {
        const obj = selectedObj();
        if(!obj || !Array.isArray(obj.linkedSlaves) || !obj.linkedSlaves[idx]) return;
        const key = phase === 'leave' ? 'leaveActions' : 'enterActions';
        if(!Array.isArray(obj.linkedSlaves[idx][key]) || !obj.linkedSlaves[idx][key][actionIdx]) return;
        obj.linkedSlaves[idx][key][actionIdx].var = sanitizeIniVarToken(value, '');
        clearPreviewSimulationCaches();
        renderLinkedSlaves();
        renderAll();
        markHistoryDirty();
    };

    window.updateLinkedSlaveActionValue = (idx, phase, actionIdx, value) => {
        const obj = selectedObj();
        if(!obj || !Array.isArray(obj.linkedSlaves) || !obj.linkedSlaves[idx]) return;
        const key = phase === 'leave' ? 'leaveActions' : 'enterActions';
        if(!Array.isArray(obj.linkedSlaves[idx][key]) || !obj.linkedSlaves[idx][key][actionIdx]) return;
        obj.linkedSlaves[idx][key][actionIdx].value = Number.isFinite(Number(value)) ? Number(value) : 0;
        clearPreviewSimulationCaches();
        renderLinkedSlaves();
        renderAll();
        markHistoryDirty();
    };

    // ========== 独立区间触发 UI 函数 ==========

    function renderRangeTriggers() {
        const obj = selectedObj();
        const container = document.getElementById('range_triggers_list');
        if(!container || !obj) return;
        container.innerHTML = '';
        const triggers = Array.isArray(obj.rangeTriggers) ? obj.rangeTriggers : [];
        if(triggers.length === 0) {
            container.innerHTML = '<div style="font-size:0.75em; color:var(--muted-color); padding:4px;">暂未配置独立区间触发，点击下方按钮添加。</div>';
            return;
        }
        triggers.forEach((trigger, idx) => {
            const srcMin = trigger.srcMin ?? 0;
            const srcMax = trigger.srcMax ?? 0.5;
            const enterActions = normalizeLinkedSlaveActionList(trigger.enterActions);
            const leaveActions = normalizeLinkedSlaveActionList(trigger.leaveActions);
            const isJoystick = obj.type === 'joystick';

            let html = `<div style="background:rgba(0,0,0,0.3); padding:6px; border:1px solid rgba(63,217,255,0.3); border-radius:4px; margin-bottom:4px;">`;
            html += `<div style="display:flex; justify-content:space-between; align-items:center; margin-bottom:4px;">
                <span style="font-size:0.8em; color:#3fd9ff; font-weight:bold;">区间触发 ${idx + 1}</span>
                <button onclick="UIB.removeRangeTrigger(${idx})" style="font-size:0.65em; padding:1px 6px; background:var(--uib-btn-danger); border:1px solid #7e3644; color:#ff5f74; cursor:pointer; border-radius:2px; width:auto;">删除</button>
            </div>`;

            if(isJoystick) {
                const pts = getRangeTriggerRegionPoints(trigger);
                html += `<div style="background:rgba(0,0,0,0.2); padding:6px; border:1px solid rgba(63,217,255,0.2); border-radius:4px; margin-bottom:3px;">
                    <div style="font-size:0.75em; color:#3fd9ff; font-weight:bold; margin-bottom:4px;">触发区域 (4 点)</div>
                    <div style="display:grid; grid-template-columns:1fr 1fr; gap:4px 6px;">`;
                const ptLabels = ['点1 (左上)', '点2 (右上)', '点3 (右下)', '点4 (左下)'];
                const ptColors = ['#ff9daa', '#7df0c0', '#9d8fff', '#ffe08a'];
                for(let pi = 0; pi < 4; pi++) {
                    const p = pts[pi] || { x: 0, y: 0 };
                    html += `<div style="border:1px solid ${ptColors[pi]}33; padding:3px; border-radius:3px;">
                        <label style="font-size:0.6em; color:${ptColors[pi]};">${ptLabels[pi]}</label>
                        <div style="display:flex; gap:3px; align-items:center;">
                            <span style="font-size:0.55em; color:var(--muted-color);">X</span>
                            <input type="number" step="0.01" min="-1" max="1" value="${p.x.toFixed(2)}"
                                onchange="UIB.updateRangeTriggerRegionPoint(${idx},${pi},'x',this.value)"
                                style="flex:1; font-size:0.7em; padding:1px; background:var(--input-bg); border:1px solid var(--border-color); color:#ff8a96; border-radius:3px;">
                            <span style="font-size:0.55em; color:var(--muted-color);">Y</span>
                            <input type="number" step="0.01" min="-1" max="1" value="${p.y.toFixed(2)}"
                                onchange="UIB.updateRangeTriggerRegionPoint(${idx},${pi},'y',this.value)"
                                style="flex:1; font-size:0.7em; padding:1px; background:var(--input-bg); border:1px solid var(--border-color); color:#a58bff; border-radius:3px;">
                        </div>
                    </div>`;
                }
                const center = getLinkedSlaveRegionCenter(pts);
                html += `</div>
                    <div style="font-size:0.62em; color:var(--muted-color); margin-top:4px; line-height:1.4;">
                        区域中心: X=${center.x.toFixed(3)} Y=${center.y.toFixed(3)}
                    </div>
                    <div style="font-size:0.62em; color:var(--muted-color); margin-top:4px; line-height:1.4;">
                        区域判定: 手柄进入 4 点构成的凸四边形时触发
                    </div>
                </div>`;
            } else {
                // 滑块组件：单轴区间
                html += `<div style="display:flex; gap:4px; margin-bottom:3px;">
                    <div style="flex:1;">
                        <label style="font-size:0.7em; color:var(--muted-color);">区间最小</label>
                        <input type="number" step="0.01" min="0" max="1" value="${srcMin}" 
                            onchange="UIB.updateRangeTriggerNum(${idx},'srcMin',this.value)" 
                            style="width:100%; font-size:0.8em; padding:2px; background:var(--input-bg); border:1px solid var(--border-color); color:var(--text-color); border-radius:6px;">
                    </div>
                    <div style="flex:1;">
                        <label style="font-size:0.7em; color:var(--muted-color);">区间最大</label>
                        <input type="number" step="0.01" min="0" max="1" value="${srcMax}"
                            onchange="UIB.updateRangeTriggerNum(${idx},'srcMax',this.value)"
                            style="width:100%; font-size:0.8em; padding:2px; background:var(--input-bg); border:1px solid var(--border-color); color:var(--text-color); border-radius:6px;">
                    </div>
                </div>`;
            }

            // 区间触发动作编辑器
            html += renderRangeTriggerActionsEditor(idx, 'enter', enterActions);
            html += renderRangeTriggerActionsEditor(idx, 'leave', leaveActions);

            html += `</div>`;
            container.innerHTML += html;
        });
    }

    function renderRangeTriggerActionsEditor(triggerIdx, phase, actions) {
        const safePhase = phase === 'leave' ? 'leave' : 'enter';
        const title = safePhase === 'leave' ? '离开区间触发' : '进入区间触发';
        const hint = safePhase === 'leave' ? '离开该区间时执行一次' : '进入该区间时执行一次';
        const rows = actions.map((action, actionIdx) => `
            <div style="display:flex; gap:4px; align-items:center; margin-top:4px;">
                <input type="text" value="${action.var || ''}" placeholder="$TargetVar"
                    onchange="UIB.updateRangeTriggerActionVar(${triggerIdx},'${safePhase}',${actionIdx},this.value)"
                    style="flex:1; font-size:0.75em; padding:2px; color:#8fe8ff; border:1px solid rgba(127,215,255,0.45); background:var(--input-bg); border-radius:4px;">
                <input type="number" step="0.01" value="${Number.isFinite(Number(action.value)) ? Number(action.value) : 0}"
                    onchange="UIB.updateRangeTriggerActionValue(${triggerIdx},'${safePhase}',${actionIdx},this.value)"
                    style="width:92px; font-size:0.75em; padding:2px; background:var(--input-bg); border:1px solid var(--border-color); color:var(--text-color); border-radius:4px;">
                <button onclick="UIB.removeRangeTriggerAction(${triggerIdx},'${safePhase}',${actionIdx})"
                    style="font-size:0.7em; padding:2px 6px; background:var(--uib-btn-danger); border:1px solid #7e3644; color:#ff5f74; cursor:pointer; border-radius:3px; width:auto;">删</button>
            </div>
        `).join('');
        return `
            <div style="margin-top:6px; padding:5px; background:rgba(63,217,255,0.06); border:1px solid rgba(63,217,255,0.22); border-radius:4px;">
                <div style="display:flex; justify-content:space-between; align-items:center; gap:6px;">
                    <div>
                        <div style="font-size:0.76em; color:#8fe8ff; font-weight:bold;">${title}</div>
                        <div style="font-size:0.65em; color:var(--muted-color);">${hint}</div>
                    </div>
                    <button onclick="UIB.addRangeTriggerAction(${triggerIdx},'${safePhase}')"
                        style="font-size:0.68em; padding:2px 8px; background:var(--uib-btn-bg); border:1px solid #2a5a72; color:#a8ecff; cursor:pointer; border-radius:3px; width:auto;">+ 参数</button>
                </div>
                ${rows || '<div style="font-size:0.68em; color:var(--muted-color); margin-top:4px;">未配置动作</div>'}
            </div>
        `;
    }

    window.addRangeTrigger = () => {
        const obj = selectedObj();
        if(!obj) return;
        if(!Array.isArray(obj.rangeTriggers)) obj.rangeTriggers = [];
        const isJoystick = obj.type === 'joystick';
        obj.rangeTriggers.push({
            srcMin: isJoystick ? -0.5 : 0,
            srcMax: isJoystick ? 0.5 : 0.5,
            srcMinY: isJoystick ? -0.5 : -1,
            srcMaxY: isJoystick ? 0.5 : 1,
            regionPoints: isJoystick ? createDefaultLinkedSlaveRegionPoints() : [],
            enterActions: [],
            leaveActions: []
        });
        renderRangeTriggers();
        markHistoryDirty();
    };

    window.removeRangeTrigger = (idx) => {
        const obj = selectedObj();
        if(!obj || !Array.isArray(obj.rangeTriggers)) return;
        obj.rangeTriggers.splice(idx, 1);
        renderRangeTriggers();
        renderAll();
        markHistoryDirty();
    };

    window.updateRangeTrigger = (idx, field, value) => {
        const obj = selectedObj();
        if(!obj || !Array.isArray(obj.rangeTriggers) || !obj.rangeTriggers[idx]) return;
        obj.rangeTriggers[idx][field] = value;
        clearPreviewSimulationCaches();
        renderRangeTriggers();
        renderAll();
        markHistoryDirty();
    };

    window.updateRangeTriggerNum = (idx, field, value) => {
        const obj = selectedObj();
        if(!obj || !Array.isArray(obj.rangeTriggers) || !obj.rangeTriggers[idx]) return;
        obj.rangeTriggers[idx][field] = parseFloat(value) || 0;
        clearPreviewSimulationCaches();
        renderRangeTriggers();
        renderAll();
        markHistoryDirty();
    };

    window.updateRangeTriggerRegionPoint = (idx, ptIdx, axis, value) => {
        const obj = selectedObj();
        if(!obj || !Array.isArray(obj.rangeTriggers) || !obj.rangeTriggers[idx]) return;
        const trigger = obj.rangeTriggers[idx];
        if(!Array.isArray(trigger.regionPoints)) trigger.regionPoints = createDefaultLinkedSlaveRegionPoints();
        if(!trigger.regionPoints[ptIdx]) trigger.regionPoints[ptIdx] = { x: 0, y: 0 };
        trigger.regionPoints[ptIdx][axis] = parseFloat(value) || 0;
        trigger.regionPoints = getRangeTriggerRegionPoints(trigger);
        if(ptIdx === 0) { trigger.srcMin = trigger.regionPoints[0].x; trigger.srcMaxY = trigger.regionPoints[0].y; }
        if(ptIdx === 1) { trigger.srcMax = trigger.regionPoints[1].x; trigger.srcMaxY = trigger.regionPoints[1].y; }
        if(ptIdx === 2) { trigger.srcMax = trigger.regionPoints[2].x; trigger.srcMinY = trigger.regionPoints[2].y; }
        if(ptIdx === 3) { trigger.srcMin = trigger.regionPoints[3].x; trigger.srcMinY = trigger.regionPoints[3].y; }
        clearPreviewSimulationCaches();
        renderRangeTriggers();
        renderAll();
        markHistoryDirty();
    };

    window.addRangeTriggerAction = (idx, phase) => {
        const obj = selectedObj();
        if(!obj || !Array.isArray(obj.rangeTriggers) || !obj.rangeTriggers[idx]) return;
        const key = phase === 'leave' ? 'leaveActions' : 'enterActions';
        if(!Array.isArray(obj.rangeTriggers[idx][key])) obj.rangeTriggers[idx][key] = [];
        obj.rangeTriggers[idx][key].push({ var: '', value: 0 });
        clearPreviewSimulationCaches();
        renderRangeTriggers();
        renderAll();
        markHistoryDirty();
    };

    window.removeRangeTriggerAction = (idx, phase, actionIdx) => {
        const obj = selectedObj();
        if(!obj || !Array.isArray(obj.rangeTriggers) || !obj.rangeTriggers[idx]) return;
        const key = phase === 'leave' ? 'leaveActions' : 'enterActions';
        if(!Array.isArray(obj.rangeTriggers[idx][key]) || !obj.rangeTriggers[idx][key][actionIdx]) return;
        obj.rangeTriggers[idx][key].splice(actionIdx, 1);
        clearPreviewSimulationCaches();
        renderRangeTriggers();
        renderAll();
        markHistoryDirty();
    };

    window.updateRangeTriggerActionVar = (idx, phase, actionIdx, value) => {
        const obj = selectedObj();
        if(!obj || !Array.isArray(obj.rangeTriggers) || !obj.rangeTriggers[idx]) return;
        const key = phase === 'leave' ? 'leaveActions' : 'enterActions';
        if(!Array.isArray(obj.rangeTriggers[idx][key]) || !obj.rangeTriggers[idx][key][actionIdx]) return;
        obj.rangeTriggers[idx][key][actionIdx].var = sanitizeIniVarToken(value, '');
        clearPreviewSimulationCaches();
        renderRangeTriggers();
        renderAll();
        markHistoryDirty();
    };

    window.updateRangeTriggerActionValue = (idx, phase, actionIdx, value) => {
        const obj = selectedObj();
        if(!obj || !Array.isArray(obj.rangeTriggers) || !obj.rangeTriggers[idx]) return;
        const key = phase === 'leave' ? 'leaveActions' : 'enterActions';
        if(!Array.isArray(obj.rangeTriggers[idx][key]) || !obj.rangeTriggers[idx][key][actionIdx]) return;
        obj.rangeTriggers[idx][key][actionIdx].value = Number.isFinite(Number(value)) ? Number(value) : 0;
        clearPreviewSimulationCaches();
        renderRangeTriggers();
        renderAll();
        markHistoryDirty();
    };

    /** 更新方向模式联动的逐方向配置 */
    window.updatePerDirection = (slaveIdx, dirIdx, field, value) => {
        const obj = selectedObj();
        if(!obj || !Array.isArray(obj.linkedSlaves) || !obj.linkedSlaves[slaveIdx]) return;
        const slave = obj.linkedSlaves[slaveIdx];
        if(!Array.isArray(slave.perDirection)) slave.perDirection = [];
        if(!slave.perDirection[dirIdx]) slave.perDirection[dirIdx] = { srcMin: 0, srcMax: 0.5, overflow: 'reset' };
        if(field === 'srcMin' || field === 'srcMax') {
            slave.perDirection[dirIdx][field] = parseFloat(value) || 0;
        } else {
            slave.perDirection[dirIdx][field] = value;
        }
        clearPreviewSimulationCaches();
        renderLinkedSlaves();
        renderAll();
        markHistoryDirty();
    };

    /** 获取组件显示名 */
    function getComponentDisplayName(comp) {
        if(!comp) return '未知';
        const typeNames = {
            slider_h: '水平滑条',
            slider_v: '垂直滑条',
            joystick: '摇杆',
            toggle: '开关',
            accum: '积蓄条',
            static: '静态',
            sequence: '序列',
            text: '文本'
        };
        const typeName = typeNames[comp.type] || comp.type;
        const firstVar = Array.isArray(comp.vars) && comp.vars[0] ? comp.vars[0] : '';
        const shortId = comp.id ? comp.id.slice(-6) : '';
        return `${typeName} ${firstVar || shortId}`;
    }

    window.addVar = () => {
        let o = selectedObj();
        if(o && o.type === 'toggle') {
            o.vars.push('');
            renderVarInputs(o);
        } else if(isSliderGridMode(o)) {
            if(!Array.isArray(o.vars)) o.vars = [''];
            o.vars.push('');
            renderVarInputs(o);
        } else {
            // 其他类型：为第一个槽添加一行
            addVarRow(0);
        }
    };
    window.removeVar = (index) => {
        let o = selectedObj();
        if(o && o.type === 'toggle') {
            o.vars.splice(index, 1);
            renderVarInputs(o);
        } else if(isSliderGridMode(o)) {
            if(!Array.isArray(o.vars) || o.vars.length <= 1) {
                o.vars = [''];
            } else {
                o.vars.splice(index, 1);
            }
            renderVarInputs(o);
        } else {
            // 其他类型：移除第一个槽的对应行
            removeVarRow(0, index);
        }
    };

    function renderSliderSubdivisionInputs(obj) {
        ensureSliderSubdivisionState(obj);
        const cfg = getSliderSubdivisionConfig(obj);
        const isBidirectional = obj.paramMode === '2';
        const defaultValue = getSliderStoredDefaultValue(obj);

        varContainer.innerHTML += `
            <div class="var-item" style="flex-wrap:wrap; padding:7px; background:rgba(0,0,0,0.2); border-radius:4px; margin-bottom:8px;">
                <div class="var-label" style="width:100%; text-align:left; color:#3fd9ff; font-weight:bold;">${isBidirectional ? '双向分段设置' : '单向分段设置'}</div>
                <div style="width:100%; font-size:0.75em; color:#8fe8ff; line-height:1.5; margin:4px 0 7px;">每段输出固定为 0–1；进入下一段后，之前的段保持为 1。</div>
                <div style="display:flex; gap:5px; width:100%; flex-wrap:wrap;">
                    <div style="flex:1; min-width:90px;">
                        <label style="font-size:0.75em; color:var(--muted-color);">${isBidirectional ? '默认位置 (0–1)' : '默认值'}</label>
                        <input type="number" step="0.01" ${isBidirectional ? 'min="0" max="1"' : ''} value="${defaultValue}" onchange="UIB.updateDefVal(0,this.value)" style="width:100%; font-size:0.85em; padding:2px;">
                    </div>
                    ${!isBidirectional ? `
                    <div style="flex:1; min-width:90px;">
                        <label style="font-size:0.75em; color:var(--muted-color);">滑块最小值</label>
                        <input type="number" step="0.01" value="${getSliderRangeMin(obj)}" onchange="UIB.updateMinVal(0,this.value)" style="width:100%; font-size:0.85em; padding:2px;">
                    </div>
                    <div style="flex:1; min-width:90px;">
                        <label style="font-size:0.75em; color:var(--muted-color);">滑块最大值</label>
                        <input type="number" step="0.01" value="${getSliderRangeMax(obj)}" onchange="UIB.updateMaxVal(0,this.value)" style="width:100%; font-size:0.85em; padding:2px;">
                    </div>` : ''}
                </div>
            </div>`;

        for(let sideIdx = 0; sideIdx < cfg.sideCount; sideIdx++) {
            const sideLabel = getSliderSubdivisionSideLabel(obj, sideIdx);
            varContainer.innerHTML += `
                <div style="width:100%; margin-top:${sideIdx === 0 ? 0 : 10}px; padding:8px; background:rgba(63,217,255,0.08); border:1px solid rgba(63,217,255,0.35); border-radius:4px;">
                    <div style="font-size:0.92em; color:#3fd9ff; font-weight:bold; margin-bottom:8px;">${sideLabel} · ${cfg.subdivisions} 段</div>
                    <div id="slider_subdiv_group_${sideIdx}" style="display:flex; flex-direction:column; gap:6px;"></div>
                </div>`;

            const sideContainer = document.getElementById(`slider_subdiv_group_${sideIdx}`);
            for(let segIdx = 0; segIdx < cfg.subdivisions; segIdx++) {
                const flatIdx = getSliderSubdivisionVarIndex(obj, sideIdx, segIdx);
                const rangeStart = segIdx * cfg.segmentSize;
                const rangeEnd = Math.min(1, (segIdx + 1) * cfg.segmentSize);
                const rawSeg = String(obj.vars[flatIdx] || '');
                const segVars = rawSeg.split(',').map(s => s.trim());
                const targets = (obj.depTargets && obj.depTargets[flatIdx]) || [];

                sideContainer.innerHTML += `
                    <div class="var-item slider-subdivision-segment" data-side="${sideIdx}" data-segment="${segIdx}" style="flex-wrap:wrap; padding:6px; background:rgba(0,0,0,0.22); border-radius:3px; margin-bottom:0;">
                        <div class="var-label" style="width:100%; text-align:left; color:#3fd9ff; font-weight:bold;">段 ${segIdx + 1} (${rangeStart.toFixed(3)} ~ ${rangeEnd.toFixed(3)}) <span style="font-size:0.7em; color:var(--muted-color); font-weight:normal;">累计 0→1</span></div>
                        <div style="width:100%; display:flex; flex-direction:column; gap:3px; margin-bottom:4px;">
                            ${segVars.map((v, rowIdx) => `
                                <div class="var-item" style="margin-bottom:0; background:rgba(255,255,255,0.02);">
                                    <div class="var-label">${rowIdx + 1}:</div>
                                    <input type="text" class="var-input" value="${v}" oninput="UIB.updateVarRow(${flatIdx},${rowIdx},this.value)" placeholder="$${sideIdx}_${segIdx}_${rowIdx + 1}">
                                    <button onclick="UIB.removeVarRow(${flatIdx},${rowIdx})" ${segVars.length <= 1 ? 'disabled style="opacity:0.45; cursor:not-allowed;"' : ''}>x</button>
                                </div>
                            `).join('')}
                        </div>
                        <button onclick="UIB.addVarRow(${flatIdx})" style="font-size:0.7em; padding:2px 8px; background:var(--uib-btn-ok); border:1px solid #2f8a62; color:var(--uib-ok-text); cursor:pointer; border-radius:3px; white-space:nowrap; margin-bottom:4px;">+ 添加变量</button>
                        <div style="width:100%; margin-top:5px; padding-top:5px; border-top:1px dashed #444;">
                            <div style="display:flex; justify-content:space-between; align-items:center; margin-bottom:5px;">
                                <span style="font-size:0.8em; color:#ffb54d;">本段触发目标</span>
                                <button onclick="UIB.addDepTarget(${flatIdx})" style="font-size:0.7em; padding:2px 8px; background:var(--uib-btn-ok); border:1px solid #2f8a62; color:var(--uib-ok-text); cursor:pointer; border-radius:3px; white-space:nowrap;">+ 添加目标</button>
                            </div>
                            <div id="dep_targets_${flatIdx}" style="display:flex; flex-direction:column; gap:5px;"></div>
                        </div>
                    </div>`;
                setTimeout(() => renderDepTargets(flatIdx, targets), 0);
            }
        }
    }

    function renderVarInputs(obj) {
        varContainer.innerHTML = '';
        if (obj.type === 'toggle') {
            obj.vars.forEach((v, i) => {
                varContainer.innerHTML += `
                    <div class="var-item">
                        <div class="var-label">${i+1}:</div>
                        <input type="text" class="var-input" value="${v}" oninput="UIB.updateVar(${i},this.value)" placeholder="$Var">
                        <button onclick="UIB.removeVar(${i})">x</button>
                    </div>`;
            });
        } else if (isSliderGridMode(obj)) {
            normalizeSliderGridState(obj);
            let gridSteps = obj.gridSteps || 3;
            let gridDepTargets = obj.gridDepTargets || [];
            const defaultIndex = clamp(Math.round(Number((obj.defVals && obj.defVals[0]) ?? 0)), 0, Math.max(0, gridSteps - 1));
            const defaultOutputValue = getSliderGridOutputValue(obj, defaultIndex);
            const isAdvancedOpen = isGridTargetEditorOpen(obj);
            const gridValuePreview = getSliderGridValueSummary(obj);
            const gridVarRows = Array.isArray(obj.vars) && obj.vars.length > 0 ? obj.vars : [''];
            
            varContainer.innerHTML += `
                <div class="var-item" style="flex-wrap:wrap; padding:5px; background:rgba(0,0,0,0.2); border-radius:3px; margin-bottom:3px;">
                    <div class="var-label" style="width:100%; text-align:left; color:#3fd9ff; font-weight:bold;">可绑定变量名（可为多个，共享同一滑条）:</div>
                    <div style="width:100%; font-size:0.78em; color:#8fe8ff; line-height:1.45; margin-bottom:6px;">输出值：${gridValuePreview}。这些变量会同步写入同一个档位值。</div>
                    <div style="width:100%; display:flex; flex-direction:column; gap:4px; margin-bottom:6px;">
                        ${gridVarRows.map((val, idx) => `
                            <div class="var-item" style="margin-bottom:0; background:rgba(255,255,255,0.02);">
                                <div class="var-label">${idx + 1}:</div>
                                <input type="text" class="var-input" value="${val || ''}" oninput="UIB.updateVar(${idx},this.value)" placeholder="$GridVar_${idx + 1}">
                                <button onclick="UIB.removeVar(${idx})" ${gridVarRows.length <= 1 ? 'disabled style="opacity:0.45; cursor:not-allowed;"' : ''}>x</button>
                            </div>
                        `).join('')}
                    </div>
                    <div style="display:flex; gap:3px; width:100%;">
                        <div style="flex:1;">
                            <label style="font-size:0.75em; color:var(--muted-color);">默认输出值</label>
                            <input type="number" step="any" value="${defaultOutputValue}" onchange="UIB.updateGridDefaultOutput(this.value)" style="width:100%; font-size:0.85em; padding:2px;">
                        </div>
                    </div>
                </div>`;
            
            varContainer.innerHTML += `
                <div style="width:100%; margin-top:10px; padding:8px; background:rgba(255,148,64,0.1); border:1px solid #ff9440; border-radius:4px;">
                    <div style="font-size:0.9em; color:#ff9440; font-weight:bold; margin-bottom:4px;">可选格子触发目标</div>
                    <div style="font-size:0.75em; color:#ffd08a; line-height:1.45; margin-bottom:8px;">只有滑条落在某个格子时才会按顺序写入目标变量。这里的目标列表和绑定变量列表是分开的。</div>
                    <button onclick="UIB.toggleGridTargetEditor()" style="font-size:0.72em; padding:3px 8px; background:var(--uib-btn-bg); border:1px solid #2a5a72; color:#d8ecf7; cursor:pointer; border-radius:3px; margin-bottom:${isAdvancedOpen ? '8px' : '0'};">${isAdvancedOpen ? '收起格子触发目标' : '展开格子触发目标'}</button>
                    <div id="grid_targets_container" style="display:${isAdvancedOpen ? 'flex' : 'none'}; flex-direction:column; gap:5px;"></div>
                </div>`;
            
            let gridContainer = document.getElementById('grid_targets_container');
            if(isAdvancedOpen && gridContainer) {
                for(let g = 0; g < gridSteps; g++) {
                    let targets = gridDepTargets[g] || [];
                    gridContainer.innerHTML += `
                        <div style="background:rgba(0,0,0,0.3); padding:5px; border-radius:3px; border-left:3px solid #ff9440;">
                            <div style="display:flex; justify-content:space-between; align-items:center; margin-bottom:3px;">
                                <span style="font-size:0.85em; color:#ff9440; font-weight:bold;">档位 ${g + 1}（值 ${formatSliderGridNumber(getSliderGridOutputValue(obj, g))}）激活时:</span>
                                <button onclick="UIB.addGridDepTarget(${g})" style="font-size:0.65em; padding:1px 6px; background:var(--uib-btn-ok); border:1px solid #2f8a62; color:var(--uib-ok-text); cursor:pointer; border-radius:2px;">+ 添加目标</button>
                            </div>
                            <div id="grid_dep_targets_${g}" style="display:flex; flex-direction:column; gap:3px;"></div>
                        </div>`;
                }
                
                for(let g = 0; g < gridSteps; g++) {
                    setTimeout(() => renderGridDepTargets(g, gridDepTargets[g] || []), 0);
                }
            }
        } else if (isSliderSubdivisionMode(obj)) {
            renderSliderSubdivisionInputs(obj);
        } else if (obj.type === 'joystick' && obj.paramMode === '4') {
            ensureJoystickDirectionState(obj);
            const cfg = getJoystickConfig(obj);

            for(let dirIdx = 0; dirIdx < cfg.directionCount; dirIdx++) {
                const angle = getJoystickDirectionAngle(obj, dirIdx);
                const directionLabel = formatAngleLabel(angle);

                varContainer.innerHTML += `
                    <div style="width:100%; margin-top:${dirIdx === 0 ? 0 : 10}px; padding:8px; background:rgba(63,217,255,0.08); border:1px solid rgba(63,217,255,0.35); border-radius:4px;">
                        <div style="font-size:0.92em; color:#3fd9ff; font-weight:bold; margin-bottom:8px;">方向 ${dirIdx + 1} · ${directionLabel}</div>
                        <div style="font-size:0.72em; color:#8fe8ff; margin-bottom:8px;">当前方向变量会按方向值细分为 ${cfg.subdivisions} 段</div>
                        <div id="joy_dir_group_${dirIdx}" style="display:flex; flex-direction:column; gap:6px;"></div>
                    </div>`;

                const directionContainer = document.getElementById(`joy_dir_group_${dirIdx}`);
                for(let segIdx = 0; segIdx < cfg.subdivisions; segIdx++) {
                    const flatIdx = getJoystickDirectionVarIndex(obj, dirIdx, segIdx);
                    const rangeStart = segIdx * cfg.segmentSize;
                    const rangeEnd = Math.min(1, (segIdx + 1) * cfg.segmentSize);
                    const rawSeg = String(obj.vars[flatIdx] || '');
                    const segVars = rawSeg.split(',').map(s => s.trim());
                    // 始终保留所有行（包括空行），确保逐行编辑正确
                    const maxVal = (obj.maxVals && obj.maxVals[flatIdx]) || 1;
                    const targets = (obj.depTargets && obj.depTargets[flatIdx]) || [];

                    directionContainer.innerHTML += `
                        <div class="var-item" style="flex-wrap:wrap; padding:6px; background:rgba(0,0,0,0.22); border-radius:3px; margin-bottom:0;">
                            <div class="var-label" style="width:100%; text-align:left; color:#3fd9ff; font-weight:bold;">段 ${segIdx + 1} (${rangeStart.toFixed(3)} ~ ${rangeEnd.toFixed(3)}) <span style="font-size:0.7em; color:var(--muted-color); font-weight:normal;">逐行添加</span></div>
                            <div style="width:100%; display:flex; flex-direction:column; gap:3px; margin-bottom:4px;">
                                ${segVars.map((v, ri) => `
                                    <div class="var-item" style="margin-bottom:0; background:rgba(255,255,255,0.02);">
                                        <div class="var-label">${ri + 1}:</div>
                                        <input type="text" class="var-input" value="${v}" oninput="UIB.updateVarRow(${flatIdx},${ri},this.value)" placeholder="$${dirIdx}_${segIdx}_${ri + 1}">
                                        <button onclick="UIB.removeVarRow(${flatIdx},${ri})" ${segVars.length <= 1 ? 'disabled style="opacity:0.45; cursor:not-allowed;"' : ''}>x</button>
                                    </div>
                                `).join('')}
                            </div>
                            <button onclick="UIB.addVarRow(${flatIdx})" style="font-size:0.7em; padding:2px 8px; background:var(--uib-btn-ok); border:1px solid #2f8a62; color:var(--uib-ok-text); cursor:pointer; border-radius:3px; white-space:nowrap; margin-bottom:4px;">+ 添加变量</button>
                            <div style="display:flex; gap:3px; width:100%;">
                                <div style="flex:1;">
                                    <label style="font-size:0.75em; color:var(--muted-color);">最大值</label>
                                    <input type="number" step="0.1" min="0.1" value="${maxVal}" onchange="UIB.updateMaxVal(${flatIdx},this.value)" style="width:100%; font-size:0.85em; padding:2px;">
                                </div>
                            </div>
                            <div style="width:100%; margin-top:5px; padding-top:5px; border-top:1px dashed #444;">
                                <div style="display:flex; justify-content:space-between; align-items:center; margin-bottom:5px;">
                                    <span style="font-size:0.8em; color:#ffb54d;">触发目标</span>
                                    <button onclick="UIB.addDepTarget(${flatIdx})" style="font-size:0.7em; padding:2px 8px; background:var(--uib-btn-ok); border:1px solid #2f8a62; color:var(--uib-ok-text); cursor:pointer; border-radius:3px; white-space:nowrap;">+ 添加目标</button>
                                </div>
                                <div id="dep_targets_${flatIdx}" style="display:flex; flex-direction:column; gap:5px;"></div>
                            </div>
                        </div>`;

                    setTimeout(() => renderDepTargets(flatIdx, targets), 0);
                }
            }
        } else {
            let labels = [];
        if (obj.type === 'slider_h') labels = (obj.paramMode === '1' || obj.paramMode === '3') ? ['Bind Var'] : ['Left Var', 'Right Var'];
        else if (obj.type === 'slider_v') labels = (obj.paramMode === '1' || obj.paramMode === '3') ? ['Bind Var'] : ['Bottom Var', 'Top Var'];
        else labels = obj.paramMode === '2' ? ['X Var', 'Y Var'] : ['Up', 'Down', 'Left', 'Right'];
            
            let maxVals = obj.maxVals || [];
            let defVals = obj.defVals || [];
            let depTargets = obj.depTargets || [];
            
            labels.forEach((l, i) => {
                 const rawVal = String(obj.vars[i] || '');
                 const slotVars = rawVal.split(',').map(s => s.trim());
                 // 始终保留所有行（包括空行），确保逐行编辑正确
                let maxVal = maxVals[i] || 1;
                let defVal = defVals[i] !== undefined ? defVals[i] : (obj.type.includes('slider') && obj.paramMode === '2' && i === 0 ? 0.5 : 0);
                let minVal = 0;
                if(obj.type === 'joystick' && obj.paramMode === '2') {
                    defVal = i === 0 ? (obj.joystickDefaultX || 0) : (obj.joystickDefaultY || 0);
                } else if(obj.type.includes('slider') && obj.paramMode === '1' && i === 0) {
                    minVal = getSliderRangeMin(obj);
                    maxVal = getSliderRangeMax(obj);
                    defVal = getSliderStoredDefaultValue(obj);
                }
                let targets = depTargets[i] || [];
                
                varContainer.innerHTML += `
                    <div class="var-item" style="flex-wrap:wrap; padding:5px; background:rgba(0,0,0,0.2); border-radius:3px; margin-bottom:3px;">
                        <div class="var-label" style="width:100%; text-align:left; color:#3fd9ff; font-weight:bold;">${l} <span style="font-size:0.7em; color:var(--muted-color); font-weight:normal;">逐行添加</span>:</div>
                        <div style="width:100%; display:flex; flex-direction:column; gap:3px; margin-bottom:4px;">
                            ${slotVars.map((v, ri) => `
                                <div class="var-item" style="margin-bottom:0; background:rgba(255,255,255,0.02);">
                                    <div class="var-label">${ri + 1}:</div>
                                    <input type="text" class="var-input" value="${v}" oninput="UIB.updateVarRow(${i},${ri},this.value)" placeholder="$${l.replace(/[^A-Za-z]/g,'')}_${ri + 1}">
                                    <button onclick="UIB.removeVarRow(${i},${ri})" ${slotVars.length <= 1 ? 'disabled style="opacity:0.45; cursor:not-allowed;"' : ''}>x</button>
                                </div>
                            `).join('')}
                        </div>
                        <button onclick="UIB.addVarRow(${i})" style="font-size:0.7em; padding:2px 8px; background:var(--uib-btn-ok); border:1px solid #2f8a62; color:var(--uib-ok-text); cursor:pointer; border-radius:3px; white-space:nowrap; margin-bottom:4px;">+ 添加变量</button>
                        <div style="display:flex; gap:3px; width:100%; flex-wrap:${obj.type.includes('slider') && obj.paramMode === '1' && i === 0 ? 'wrap' : 'nowrap'};">
                            <div style="flex:1;">
                                <label style="font-size:0.75em; color:var(--muted-color);">默认值</label>
                                <input type="number" step="0.01" value="${defVal}" onchange="UIB.updateDefVal(${i},this.value)" style="width:100%; font-size:0.85em; padding:2px;">
                            </div>
                            ${obj.type.includes('slider') && obj.paramMode === '1' && i === 0 ? `
                            <div style="flex:1;">
                                <label style="font-size:0.75em; color:var(--muted-color);">最小值</label>
                                <input type="number" step="0.01" value="${minVal}" onchange="UIB.updateMinVal(${i},this.value)" style="width:100%; font-size:0.85em; padding:2px;">
                            </div>
                            ` : ''}
                            <div style="flex:1;">
                                <label style="font-size:0.75em; color:var(--muted-color);">${obj.type.includes('slider') && obj.paramMode === '1' && i === 0 ? '最大值' : '最大值'}</label>
                                <input type="number" step="0.1" ${obj.type.includes('slider') && obj.paramMode === '1' && i === 0 ? '' : 'min="0.1"'} value="${maxVal}" onchange="UIB.updateMaxVal(${i},this.value)" style="width:100%; font-size:0.85em; padding:2px;">
                            </div>
                        </div>
                        <div style="width:100%; margin-top:5px; padding-top:5px; border-top:1px dashed #444;">
                            <div style="display:flex; justify-content:space-between; align-items:center; margin-bottom:5px;">
                                <span style="font-size:0.8em; color:#ffb54d;">触发目标</span>
                                <button onclick="UIB.addDepTarget(${i})" style="font-size:0.7em; padding:2px 8px; background:var(--uib-btn-ok); border:1px solid #2f8a62; color:var(--uib-ok-text); cursor:pointer; border-radius:3px; white-space:nowrap;">+ 添加目标</button>
                            </div>
                            <div id="dep_targets_${i}" style="display:flex; flex-direction:column; gap:5px;">
                            </div>
                        </div>
                    </div>`;
                    
                setTimeout(() => renderDepTargets(i, targets), 0);
            });
        }
    }
    
    window.updateDefVal = (i, v) => {
        let o = selectedObj();
        if(o) {
            if(!o.defVals) o.defVals = [];
            let nextVal = parseFloat(v) || 0;
            if(o.type === 'joystick' && o.paramMode === '2') {
                nextVal = clamp(nextVal, -1, 1);
                if(i === 0) o.joystickDefaultX = nextVal;
                if(i === 1) o.joystickDefaultY = nextVal;
            } else if(o.type && o.type.includes('slider') && o.paramMode === '2' && i === 0) {
                nextVal = clamp(nextVal, 0, 1);
            } else if(o.type && o.type.includes('slider') && o.paramMode === '1' && i === 0) {
                nextVal = clampSliderActualValue(o, nextVal);
            } else if(isSliderGridMode(o) && i === 0) {
                nextVal = clamp(Math.round(nextVal), 0, Math.max(0, (o.gridSteps || 3) - 1));
            }
            o.defVals[i] = nextVal;
            clearPreviewSimulationCaches();
            renderAll();
        }
    };

    window.updateGridDefaultOutput = (value) => {
        const o = selectedObj();
        if(!isSliderGridMode(o)) return;
        if(!o.defVals) o.defVals = [];
        o.defVals[0] = getSliderGridIndexFromOutputValue(o, value);
        clearPreviewSimulationCaches();
        renderVarInputs(o);
        renderAll();
    };

    window.updateMinVal = (i, v) => {
        let o = selectedObj();
        if(!o || !o.type || !o.type.includes('slider') || o.paramMode !== '1') return;
        if(!o.minVals) o.minVals = [];
        if(!o.maxVals) o.maxVals = [];
        let nextMin = parseFloat(v);
        if(!Number.isFinite(nextMin)) nextMin = 0;
        const currentMax = Number.isFinite(Number(o.maxVals[i])) ? Number(o.maxVals[i]) : 1;
        o.minVals[i] = nextMin;
        if(currentMax < nextMin) o.maxVals[i] = nextMin;
        if(!o.defVals) o.defVals = [];
        o.defVals[i] = clampSliderActualValue(o, o.defVals[i]);
        clearPreviewSimulationCaches();
        updatePropPanel();
        renderAll();
    };
    
    window.updateMaxVal = (i, v) => {
        let o = selectedObj();
        if(o) {
            if(!o.maxVals) o.maxVals = [];
            let nextVal = parseFloat(v);
            if(!Number.isFinite(nextVal)) nextVal = 1;
            if(o.type && o.type.includes('slider') && o.paramMode === '1' && i === 0) {
                const minVal = getSliderRangeMin(o);
                o.maxVals[i] = nextVal < minVal ? minVal : nextVal;
                if(!o.defVals) o.defVals = [];
                o.defVals[i] = clampSliderActualValue(o, o.defVals[i]);
            } else {
                o.maxVals[i] = Math.max(0.1, nextVal);
            }
            clearPreviewSimulationCaches();
            renderAll();
        }
    };
    
    window.renderDepTargets = (paramIdx, targets) => {
        let container = document.getElementById(`dep_targets_${paramIdx}`);
        if(!container) return;
        container.innerHTML = '';
        
        let o = selectedObj();
        let isJoystick4 = o && o.type === 'joystick' && o.paramMode === '4';
        let joystickCfg = isJoystick4 ? getJoystickConfig(o) : null;
        
        targets.forEach((t, ti) => {
            let targetVar = t.var || '';
            let inverted = t.invert || false;
            let useElse = t.else || false;
            let trueVal = inverted ? 0 : 1;
            let falseVal = inverted ? 1 : 0;
            
            let condText;
            if(isJoystick4) {
                const dirIdx = Math.floor(paramIdx / joystickCfg.subdivisions);
                const segIdx = paramIdx % joystickCfg.subdivisions;
                const angle = getJoystickDirectionAngle(o, dirIdx);
                const rangeStart = segIdx * joystickCfg.segmentSize;
                const rangeEnd = Math.min(1, (segIdx + 1) * joystickCfg.segmentSize);
                condText = `${formatAngleLabel(angle)} 方向值 ${rangeStart.toFixed(3)} ~ ${rangeEnd.toFixed(3)}`;
            } else {
                condText = '>= 0.001';
            }
            
            container.innerHTML += `
                <div class="dep-target-item" style="background:rgba(0,0,0,0.3); padding:5px; border-radius:3px; border-left:3px solid #ffb54d;">
                    <div style="display:flex; gap:5px; margin-bottom:3px;">
                        <input type="text" value="${targetVar}" placeholder="$TargetVar" onchange="UIB.updateDepTargetVar(${paramIdx}, ${ti}, this.value)" style="flex:1; font-size:0.8em; padding:2px; color:#ffb54d; border-color:#ff9440;">
                        <button onclick="UIB.removeDepTarget(${paramIdx}, ${ti})" style="font-size:0.7em; padding:2px 6px; background:var(--uib-btn-danger); border:1px solid #7e3644; color:#ff5f74; cursor:pointer; border-radius:3px;">删</button>
                    </div>
                    <div style="display:flex; gap:10px; flex-wrap:wrap;">
                        <label style="font-size:0.7em; color:var(--muted-color); display:flex; align-items:center; gap:3px;">
                            <input type="checkbox" ${inverted ? 'checked' : ''} onchange="UIB.updateDepTargetInvert(${paramIdx}, ${ti}, this.checked)" style="width:1em; height:1em;">
                            <span style="color:#ff5f74;">反转</span>
                        </label>
                        <label style="font-size:0.7em; color:var(--muted-color); display:flex; align-items:center; gap:3px;">
                            <input type="checkbox" ${useElse ? 'checked' : ''} onchange="UIB.updateDepTargetElse(${paramIdx}, ${ti}, this.checked)" style="width:1em; height:1em;">
                            <span style="color:#3fd9ff;">双态模式</span>
                        </label>
                    </div>
                    <div style="font-size:0.65em; color:var(--muted-color); margin-top:2px;">
                        ${useElse ? `${condText} -> ${trueVal}, 否则 -> ${falseVal}` : `${condText} -> ${trueVal}`}
                    </div>
                </div>`;
        });
    };
    
    window.addDepTarget = (paramIdx) => {
        let o = selectedObj();
        if(!o) return;
        if(!o.depTargets) o.depTargets = [];
        if(!o.depTargets[paramIdx]) o.depTargets[paramIdx] = [];
        o.depTargets[paramIdx].push({ var: '', invert: false, else: false });
        renderDepTargets(paramIdx, o.depTargets[paramIdx]);
    };
    
    window.removeDepTarget = (paramIdx, targetIdx) => {
        let o = selectedObj();
        if(!o || !o.depTargets || !o.depTargets[paramIdx]) return;
        o.depTargets[paramIdx].splice(targetIdx, 1);
        renderDepTargets(paramIdx, o.depTargets[paramIdx]);
    };
    
    window.updateDepTargetVar = (paramIdx, targetIdx, val) => {
        let o = selectedObj();
        if(!o || !o.depTargets || !o.depTargets[paramIdx]) return;
        o.depTargets[paramIdx][targetIdx].var = sanitizeIniVarToken(val, '');
        renderDepTargets(paramIdx, o.depTargets[paramIdx]);
    };
    
    window.updateDepTargetInvert = (paramIdx, targetIdx, inverted) => {
        let o = selectedObj();
        if(!o || !o.depTargets || !o.depTargets[paramIdx]) return;
        o.depTargets[paramIdx][targetIdx].invert = inverted;
        renderDepTargets(paramIdx, o.depTargets[paramIdx]);
    };
    
    window.updateDepTargetElse = (paramIdx, targetIdx, useElse) => {
        let o = selectedObj();
        if(!o || !o.depTargets || !o.depTargets[paramIdx]) return;
        o.depTargets[paramIdx][targetIdx].else = useElse;
        renderDepTargets(paramIdx, o.depTargets[paramIdx]);
    };
    
    window.renderGridDepTargets = (gridIdx, targets) => {
        let container = document.getElementById(`grid_dep_targets_${gridIdx}`);
        if(!container) return;
        container.innerHTML = '';
        
        targets.forEach((t, ti) => {
            let targetVar = t.var || '';
            let inverted = t.invert || false;
            let useElse = t.else !== false;
            let trueVal = inverted ? 0 : 1;
            let falseVal = inverted ? 1 : 0;
            
            container.innerHTML += `
                <div style="display:flex; gap:3px; align-items:center; background:rgba(0,0,0,0.2); padding:3px; border-radius:2px;">
                    <input type="text" value="${targetVar}" placeholder="$Target" onchange="UIB.updateGridDepTargetVar(${gridIdx}, ${ti}, this.value)" style="flex:1; font-size:0.75em; padding:2px; color:#ff9440; border-color:#ff9440;">
                    <label style="font-size:0.6em; display:flex; align-items:center; gap:1px; color:var(--muted-color);" title="双态模式：离开当前格子时恢复为 0">
                        <input type="checkbox" ${useElse ? 'checked' : ''} onchange="UIB.updateGridDepTargetElse(${gridIdx}, ${ti}, this.checked)" style="width:1em; height:1em;">
                        双态
                    </label>
                    <label style="font-size:0.6em; display:flex; align-items:center; gap:1px; color:var(--muted-color);" title="反转：激活时写入 0，未激活时写入 1">
                        <input type="checkbox" ${inverted ? 'checked' : ''} onchange="UIB.updateGridDepTargetInvert(${gridIdx}, ${ti}, this.checked)" style="width:1em; height:1em;">
                        反转
                    </label>
                <button onclick="UIB.removeGridDepTarget(${gridIdx}, ${ti})" style="font-size:0.6em; padding:1px 4px; background:var(--uib-btn-danger); border:1px solid #7e3644; color:#ff5f74; cursor:pointer; border-radius:2px;">删</button>
                </div>`;
        });
    };

    window.toggleGridTargetEditor = () => {
        let o = selectedObj();
        if(!isSliderGridMode(o)) return;
        setGridTargetEditorOpen(o, !isGridTargetEditorOpen(o));
        renderVarInputs(o);
    };
    
    window.addGridDepTarget = (gridIdx) => {
        let o = selectedObj();
        if(!o) return;
        if(!o.gridDepTargets) o.gridDepTargets = [];
        if(!o.gridDepTargets[gridIdx]) o.gridDepTargets[gridIdx] = [];
        setGridTargetEditorOpen(o, true);
        o.gridDepTargets[gridIdx].push({ var: '', invert: false, else: true });
        renderVarInputs(o);
    };
    
    window.removeGridDepTarget = (gridIdx, targetIdx) => {
        let o = selectedObj();
        if(!o || !o.gridDepTargets || !o.gridDepTargets[gridIdx]) return;
        o.gridDepTargets[gridIdx].splice(targetIdx, 1);
        renderGridDepTargets(gridIdx, o.gridDepTargets[gridIdx]);
    };
    
    window.updateGridDepTargetVar = (gridIdx, targetIdx, val) => {
        let o = selectedObj();
        if(!o || !o.gridDepTargets || !o.gridDepTargets[gridIdx]) return;
        o.gridDepTargets[gridIdx][targetIdx].var = sanitizeIniVarToken(val, '');
        renderGridDepTargets(gridIdx, o.gridDepTargets[gridIdx]);
    };
    
    window.updateGridDepTargetInvert = (gridIdx, targetIdx, inverted) => {
        let o = selectedObj();
        if(!o || !o.gridDepTargets || !o.gridDepTargets[gridIdx]) return;
        o.gridDepTargets[gridIdx][targetIdx].invert = inverted;
        renderGridDepTargets(gridIdx, o.gridDepTargets[gridIdx]);
    };
    
    window.updateGridDepTargetElse = (gridIdx, targetIdx, useElse) => {
        let o = selectedObj();
        if(!o || !o.gridDepTargets || !o.gridDepTargets[gridIdx]) return;
        o.gridDepTargets[gridIdx][targetIdx].else = useElse;
        renderGridDepTargets(gridIdx, o.gridDepTargets[gridIdx]);
    };
    
    function getComponentResourceParts(obj, bgOnly=false) {
        const parts = {};
        const hasBackground = obj.type !== 'static' && obj.type !== 'sequence' && obj.type !== 'toggle';
        if(hasBackground) parts.bg = '背景';
        if(!bgOnly) {
            if(obj.type === 'static') {
                parts.img = '图片';
            } else if(obj.type === 'joystick') {
                parts.handle = '摇杆头';
                parts.post = '碰撞桩';
            } else if(obj.type === 'toggle') {
                if(isToggleMultiMode(obj)) {
                    parts.prog_off = '未激活';
                    parts.prog_on = '已激活';
                } else {
                    parts.off = '关闭态';
                    parts.on = '开启态';
                }
            } else if(obj.type === 'slider_h' || obj.type === 'slider_v') {
                parts.handle = '滑块';
                parts.bar_l = '填充';
                parts.bar_r = '轨道';
            } else if(obj.type === 'accum') {
                parts.bar_l = '红色积蓄';
                parts.bar_r = '蓝色轨道';
            }
        }
        return parts;
    }

    function getComponentResourcePartLabel(obj, key) {
        if(key === 'post_marker') return '碰撞桩';
        return getComponentResourceParts(obj, false)[key] || key;
    }

    function collectWorkspaceResourceUsage(componentList = components) {
        const merged = new Map();
        const addUsage = (component, kind, rawPath, previewUrl = '') => {
            const normalizedPath = normalizeZipAssetPath(rawPath);
            if(!normalizedPath) return;
            const dedupeKey = normalizedPath.toLocaleLowerCase('en-US');
            let entry = merged.get(dedupeKey);
            if(!entry) {
                entry = {
                    path: normalizedPath,
                    previewUrl: previewUrl || '',
                    kinds: new Set(),
                    componentNames: new Set(),
                    usageCount: 0
                };
                merged.set(dedupeKey, entry);
            }
            if(!entry.previewUrl && previewUrl) entry.previewUrl = previewUrl;
            entry.kinds.add(kind);
            entry.componentNames.add(getComponentDisplayName(component));
            entry.usageCount += 1;
        };

        (componentList || []).forEach(component => {
            if(!component) return;
            Object.entries(component.paths || {}).forEach(([key, rawPath]) => {
                const embeddedPreview = component.embeddedAssets && component.embeddedAssets[key]
                    ? component.embeddedAssets[key].dataUrl
                    : '';
                const previewUrl = (component.preview && component.preview[key]) || embeddedPreview || '';
                addUsage(component, getComponentResourcePartLabel(component, key), rawPath, previewUrl);
            });
            if(component.type === 'sequence') {
                (component.frames || []).forEach(frame => {
                    if(!frame) return;
                    addUsage(component, `序列帧 ${frame.val ?? ''}`.trim(), frame.path, frame.preview || frame.dataUrl || '');
                });
            }
        });

        return Array.from(merged.values()).sort((a, b) => a.path.localeCompare(b.path, 'zh-CN', {numeric: true, sensitivity: 'base'}));
    }

    function renderGlobalResourcePreview() {
        if(!resContainer) return;
        const entries = collectWorkspaceResourceUsage();
        if(resourceWindowTitle) resourceWindowTitle.textContent = '资源窗口';
        if(resourceScopeLabel) {
            resourceScopeLabel.innerHTML = `<strong>全局预览</strong> · ${entries.length} 个已合并资源 · 未选择组件`;
        }
        resContainer.innerHTML = '';
        if(entries.length === 0) {
            const empty = document.createElement('div');
            empty.className = 'anim-note';
            empty.textContent = '当前工作区还没有已设置的资源。';
            resContainer.appendChild(empty);
            return;
        }

        const list = document.createElement('div');
        list.className = 'resource-global-list';
        entries.forEach(entry => {
            const row = document.createElement('div');
            row.className = 'resource-global-row';

            const preview = document.createElement('div');
            preview.className = entry.previewUrl ? 'res-preview' : 'res-preview empty';
            if(entry.previewUrl) preview.style.backgroundImage = `url("${String(entry.previewUrl).replace(/"/g, '%22')}")`;
            else preview.textContent = '无预览';

            const info = document.createElement('div');
            const path = document.createElement('code');
            path.className = 'resource-global-path';
            path.textContent = entry.path;
            const meta = document.createElement('div');
            meta.className = 'resource-global-meta';
            meta.textContent = `${Array.from(entry.kinds).join('、')} · 使用 ${entry.usageCount} 次 · ${Array.from(entry.componentNames).join('、')}`;
            info.append(path, meta);
            row.append(preview, info);
            list.appendChild(row);
        });
        resContainer.appendChild(list);
    }

    function renderSequenceResourceInputs(obj) {
        const frames = obj && Array.isArray(obj.frames) ? obj.frames : [];
        const section = document.createElement('div');
        section.className = 'input-group';
        const heading = document.createElement('h4');
        heading.textContent = '序列帧资源';
        section.appendChild(heading);
        if(frames.length === 0) {
            const empty = document.createElement('div');
            empty.className = 'anim-note';
            empty.textContent = '此组件还没有序列帧。';
            section.appendChild(empty);
        }
        frames.forEach((frame, index) => {
            const previewUrl = frame.preview || frame.dataUrl || '';
            const row = document.createElement('div');
            row.className = 'res-row resource-drop-target';
            row.title = '可将图片文件拖入此处';
            row.ondragover = handleResourceDragOver;
            row.ondragleave = handleResourceDragLeave;
            row.ondrop = event => dropSequenceFrame(event, index);
            row.innerHTML = `
                <label class="res-label">帧 ${escapeHtml(frame.val ?? index)}</label>
                <div class="${previewUrl ? 'res-preview' : 'res-preview empty'}"${previewUrl ? ` style="background-image:url(${previewUrl})"` : ''}>${previewUrl ? '' : '无预览'}</div>
                <input type="text" class="res-path" value="${escapeHtml(frame.path || '')}" readonly>
                <span class="res-alpha" title="序列帧索引">#${index + 1}</span>
                <input type="file" id="resource_seq_upload_${index}" style="display:none" accept="image/*,.dds,.tga" onchange="UIB.uploadSeqFrame(${index}, this)">
                <button class="res-upload" onclick="document.getElementById('resource_seq_upload_${index}').click()">上传</button>`;
            section.appendChild(row);
        });
        resContainer.appendChild(section);
    }

    function renderResInputs(obj, bgOnly=false) {
        resContainer.innerHTML = '';
        const parts = getComponentResourceParts(obj, bgOnly);

        for(const k in parts) {
            const previewUrl = obj.preview && obj.preview[k] ? obj.preview[k] : '';
            const previewClass = previewUrl ? 'res-preview' : 'res-preview empty';
            const previewStyle = previewUrl ? ` style="background-image:url(${previewUrl})"` : '';
            const rawPathValue = (obj.paths && obj.paths[k]) || (k === 'post' ? ((obj.paths && obj.paths.post_marker) || '') : '');
            const pathValue = escapeHtml(rawPathValue);
            const opacityValue = getComponentResourceOpacity(obj, k).toFixed(3);
            resContainer.innerHTML += `
                <div class="res-row resource-drop-target" title="可将图片文件拖入此处" ondragover="UIB.handleResourceDragOver(event)" ondragleave="UIB.handleResourceDragLeave(event)" ondrop="UIB.dropTexture(event,'${k}')">
                    <label class="res-label">${parts[k]}</label>
                    <div class="${previewClass}"${previewStyle}>${previewUrl ? '' : '无预览'}</div>
                    <input type="text" class="res-path" value="${pathValue}" oninput="UIB.updatePath('${k}',this.value,this)" placeholder="可直接填写资源文件名">
                    <input type="number" class="res-alpha" value="${opacityValue}" step="0.01" min="0" max="1" onchange="UIB.updateResourceOpacity('${k}',this.value)" title="透明度">
                    <input type="file" id="u_${k}" style="display:none" accept="image/*,.dds,.tga" onchange="UIB.uploadTex('${k}',this)">
                    <button class="res-upload" onclick="document.getElementById('u_${k}').click()">上传</button>
                </div>`;
        }
    }

    function refreshResourceWindow(obj = selectedObj()) {
        if(!resContainer) return;
        if(!obj) {
            renderGlobalResourcePreview();
            return;
        }
        if(resourceWindowTitle) resourceWindowTitle.textContent = '资源窗口';
        if(resourceScopeLabel) {
            resourceScopeLabel.innerHTML = `<strong>当前组件</strong> · ${escapeHtml(getComponentDisplayName(obj))}`;
        }
        renderResInputs(obj, obj.type === 'sequence' || obj.type === 'text');
        if(obj.type === 'sequence') renderSequenceResourceInputs(obj);
    }

    window.updateGeom = () => { 
        const group = selectedGroup();
        if(group) {
            const bounds = getGroupNormalizedBounds(group.id);
            if(!bounds) return;
            let newX = parseFloat(document.getElementById('p_x').value);
            let newY = parseFloat(document.getElementById('p_y').value);
            const dx = (Number.isFinite(newX) ? newX : bounds.x) - bounds.x;
            const dy = (Number.isFinite(newY) ? newY : bounds.y) - bounds.y;
            if(Math.abs(dx) > 0.000001 || Math.abs(dy) > 0.000001) {
                getDescendantComponents(makeEntityRef('group', group.id)).forEach(component => {
                    component.x += dx;
                    component.y += dy;
                });
            }
            renderAll();
            return;
        }
        let o = selectedObj(); 
        if(o){ 
            const oldX = o.x, oldY = o.y;
            const oldRect = { x: o.x, y: o.y, w: o.w, h: o.h };
            let newX = parseFloat(document.getElementById('p_x').value); 
            let newY = parseFloat(document.getElementById('p_y').value); 
            let newW = parseFloat(document.getElementById('p_w').value); 
            let newH = parseFloat(document.getElementById('p_h').value); 
            const newZ = parseInt(document.getElementById('p_z').value) || 0;
            const newRot = parseFloat(document.getElementById('p_rot').value) || 0;
            const newCornerRadius = Math.max(0, parseFloat(document.getElementById('p_corner_radius').value) || 0);
            const snapCfg = getGridSnapConfig();
            if(snapCfg.enabled) {
                const snappedRect = snapRectToGrid({ x: newX, y: newY, w: newW, h: newH }, snapCfg, oldRect);
                newX = snappedRect.x;
                newY = snappedRect.y;
                newW = snappedRect.w;
                newH = snappedRect.h;
            }
            o.x = newX; o.y = newY; o.w = newW; o.h = newH; o.zIndex = newZ; o.rot = newRot; o.cornerRadius = newCornerRadius;
            refreshGeomInputs(o);
            renderAll(); 
        } 
    };
    window.updateVis = () => { let o = selectedObj(); if(o){ o.handleSize=parseFloat(document.getElementById('p_hs').value); o.trackThick=parseFloat(document.getElementById('p_tt').value); renderAll(); } };
    window.updateGroupProps = () => {
        const group = selectedGroup();
        if(!group) return;
        group.name = sanitizeGroupDisplayName((document.getElementById('g_name').value || '').trim(), group.name || group.id);
        document.getElementById('g_name').value = group.name;
        group.visVar = sanitizeIniVarToken(document.getElementById('g_vis_var').value, '');
        group.visDefault = document.getElementById('g_vis_default').checked;
        group.pinned = document.getElementById('g_pinned').checked;
        group.bindingEnabled = document.getElementById('g_binding_enabled').checked;
        document.getElementById('g_vis_var').value = group.visVar;
        renderHierarchyPanel();
        renderAll();
    };
    window.updateTextProps = () => { 
        let o = selectedObj(); 
        if(o){ 
            o.charSize=parseFloat(document.getElementById('p_char_size').value); 
            o.lineGap=parseFloat(document.getElementById('p_line_gap').value); 
            o.valVar=sanitizeIniVarToken(document.getElementById('p_val_var').value, '');
            document.getElementById('p_val_var').value = o.valVar;
            o.fontFamily = document.getElementById('p_font_family').value;
            o.fontBold = document.getElementById('p_font_bold').checked;
            o.fontItalic = document.getElementById('p_font_italic').checked;
            o.textFlow = document.getElementById('p_text_flow').value === 'vertical' ? 'vertical' : 'horizontal';
            o.textVisibilityEnabled = document.getElementById('p_text_visibility_enabled').checked;
            o.visVar = sanitizeIniVarToken(document.getElementById('p_text_vis_var').value, getDefaultTextVisibilityVar(o));
            o.visDefault = document.getElementById('p_text_vis_default').checked;
            o.textHoverEffect = document.getElementById('p_text_hover_effect').checked;
            o.textClickVar = sanitizeIniVarToken(document.getElementById('p_text_click_var').value, '');
            previewTextVariableStates.clear();
            document.getElementById('p_text_vis_var').value = o.visVar;
            document.getElementById('p_text_click_var').value = o.textClickVar;
            document.getElementById('p_text_vis_var').disabled = !o.textVisibilityEnabled;
            document.getElementById('p_text_vis_default').disabled = !o.textVisibilityEnabled;
            document.getElementById('p_text_vis_var_name').innerText = o.textVisibilityEnabled ? o.visVar : '未启用';
            renderAll(); 
        } 
    };
    window.updateTextContent = () => { let o = selectedObj(); if(o){ o.textContent=document.getElementById('p_text_content').value; renderAll(); } };
    function updateGlobalAnimValue(assigner) {
        const target = getAnimationEditorTarget();
        if(!target) return;
        const holder = target.entity;
        normalizeAnimationState(holder);
        assigner(holder.globalAnim);
        normalizeAnimationState(holder);
        refreshAnimationHints(target.component || selectedObj(), target);
        renderAll();
    }
    function updateLocalAnimValue(assigner) {
        const o = selectedObj();
        if(!o) return;
        normalizeAnimationState(o);
        assigner(o.localAnim);
        normalizeAnimationState(o);
        refreshAnimationHints(o);
        renderAll();
    }
    window.updateGlobalAnimMode = () => {
        updateGlobalAnimValue(anim => { anim.mode = document.getElementById('p_anim_global_mode').value; });
    };
    window.updateGlobalAnimEdge = () => {
        updateGlobalAnimValue(anim => { anim.edge = document.getElementById('p_anim_global_edge').value; });
    };
    window.updateGlobalAnimStrength = () => {
        updateGlobalAnimValue(anim => { anim.strength = parseFloat(document.getElementById('p_anim_global_strength').value); });
    };
    window.updateGlobalAnimSpeed = () => {
        updateGlobalAnimValue(anim => { anim.speed = parseFloat(document.getElementById('p_anim_global_speed').value); });
    };
    window.updateGlobalAnimReveal = () => {
        updateGlobalAnimValue(anim => { anim.reveal = parseFloat(document.getElementById('p_anim_global_reveal').value); });
    };
    window.updateGlobalAnimTrigger = () => {
        updateGlobalAnimValue(anim => { anim.trigger = parseFloat(document.getElementById('p_anim_global_trigger').value); });
    };
    window.updateGlobalAnimEase = () => {
        updateGlobalAnimValue(anim => { anim.ease = parseFloat(document.getElementById('p_anim_global_ease').value); });
    };
    window.updateLocalAnimMode = () => {
        updateLocalAnimValue(anim => { anim.mode = document.getElementById('p_anim_local_mode').value; });
    };
    window.updateLocalAnimStrength = () => {
        updateLocalAnimValue(anim => { anim.strength = parseFloat(document.getElementById('p_anim_local_strength').value); });
    };
    window.updateLocalAnimSpeed = () => {
        updateLocalAnimValue(anim => { anim.speed = parseFloat(document.getElementById('p_anim_local_speed').value); });
    };
    window.updatePersistentAnimSpeed = () => {
        getPersistentAnimSettings();
        renderAll();
    };
    window.updatePersistentAnimEnabled = () => {
        getPersistentAnimSettings();
        renderAll();
    };
    window.updatePersistentAnimFlowSpeed = () => {
        getPersistentAnimSettings();
        renderAll();
    };
    window.updateData = () => { 
        let o = selectedObj(); 
        if(o){ 
            o.physics=document.getElementById('p_phys').checked; 
            if(o.type === 'toggle') {
                if(isToggleMultiMode(o)) o.initialValue = parseInt(document.getElementById('p_initial_num').value, 10);
                else {
                    o.initialValue = document.getElementById('p_initial_val').checked ? 1 : 0;
                    o.toggleInvert = document.getElementById('p_toggle_invert').checked === true;
                }
                normalizeToggleState(o);
            }
            updatePropPanel(); renderAll(); 
        } 
    };
    
    window.updatePhysParamsDirect = () => {
        let o = selectedObj();
        if(o) {
            let kVal = parseFloat(document.getElementById('p_spring_k_val').value) || 0;
            let dVal = parseFloat(document.getElementById('p_spring_d_val').value) || 0;
            if(kVal < 0) kVal = 0;
            if(kVal > 1) kVal = 1;
            if(dVal < 0) dVal = 0;
            if(dVal > 1) dVal = 1;
            document.getElementById('p_spring_k_val').value = kVal;
            document.getElementById('p_spring_d_val').value = dVal;
            o.springK = kVal;
            o.springD = dVal;
        }
    };

    window.updatePhysProfile = () => {
        let o = selectedObj();
        if(!o) return;
        o.physicsProfile = document.getElementById('p_phys_profile').value === 'breast' ? 'breast' : 'normal';
        document.getElementById('phys_profile_hint').style.display = (o.type === 'joystick' && o.physicsProfile === 'breast' && o.physics) ? 'block' : 'none';
    };
    
    window.updateAutoAnimate = () => {
        let o = selectedObj();
        if(o) {
            o.autoAnimate = document.getElementById('p_auto_animate').checked;
            let idx = components.findIndex(c => c.id === o.id);
            document.getElementById('auto_var_name').innerText = idx >= 0 ? `$auto_${idx}` : '';
            const autoAnimSection = document.getElementById('auto_anim_section');
            if(autoAnimSection && o.autoAnimate) autoAnimSection.open = true;
        }
    };
    
    window.updateAutoStr = () => {
        let o = selectedObj();
        if(o) {
            let v = parseFloat(document.getElementById('p_auto_str').value) || 0;
            if(v < 0) v = 0;
            if(v > 1) v = 1;
            document.getElementById('p_auto_str').value = v;
            o.autoStr = v;
        }
    };

    window.updateAutoAmpX = () => {
        let o = selectedObj();
        if(o) {
            let v = parseFloat(document.getElementById('p_auto_amp_x').value);
            if(!Number.isFinite(v)) v = 1;
            v = clamp(v, 0, 3);
            document.getElementById('p_auto_amp_x').value = v;
            o.autoAmpX = v;
            renderAutoFunctionPreview(o);
        }
    };

    window.updateAutoAmpY = () => {
        let o = selectedObj();
        if(o) {
            let v = parseFloat(document.getElementById('p_auto_amp_y').value);
            if(!Number.isFinite(v)) v = 1;
            v = clamp(v, 0, 3);
            document.getElementById('p_auto_amp_y').value = v;
            o.autoAmpY = v;
            renderAutoFunctionPreview(o);
        }
    };

    window.updateAutoSource = () => {
        let o = selectedObj();
        if(o) {
            o.autoSource = document.getElementById('p_auto_source').value === 'function' ? 'function' : 'chaos';
            applyAutoEditorState(o);
        }
    };

    window.updateAutoSeedX = () => {
        let o = selectedObj();
        if(o) {
            let v = parseFloat(document.getElementById('p_auto_seed_x').value);
            if(!Number.isFinite(v)) v = 0.3187;
            v = clamp(v, 0.001, 0.999);
            document.getElementById('p_auto_seed_x').value = v;
            o.autoSeedX = v;
            renderAutoFunctionPreview(o);
        }
    };

    window.updateAutoSeedY = () => {
        let o = selectedObj();
        if(o) {
            let v = parseFloat(document.getElementById('p_auto_seed_y').value);
            if(!Number.isFinite(v)) v = 0.6123;
            v = clamp(v, 0.001, 0.999);
            document.getElementById('p_auto_seed_y').value = v;
            o.autoSeedY = v;
            renderAutoFunctionPreview(o);
        }
    };

    window.updateAutoFuncX = () => {
        let o = selectedObj();
        if(!o) return;
        const input = document.getElementById('p_auto_func_x');
        const fallback = o.type === 'joystick' ? 'sin(TAU * t)' : 'sin01(t)';
        const next = (input.value || '').trim() || fallback;
        try {
            compileAutoExpression(next);
            input.value = next;
            input.title = '';
            input.style.borderColor = '#2a5a72';
            o.autoFuncX = next;
            renderAutoFunctionPreview(o);
        } catch(err) {
            input.value = o.autoFuncX || fallback;
            input.title = err.message;
            input.style.borderColor = '#a84a56';
            alert('Invalid auto function X.');
        }
    };

    window.updateAutoFuncY = () => {
        let o = selectedObj();
        if(!o) return;
        const input = document.getElementById('p_auto_func_y');
        const fallback = 'cos(TAU * t)';
        const next = (input.value || '').trim() || fallback;
        try {
            compileAutoExpression(next);
            input.value = next;
            input.title = '';
            input.style.borderColor = '#2a5a72';
            o.autoFuncY = next;
            renderAutoFunctionPreview(o);
        } catch(err) {
            input.value = o.autoFuncY || fallback;
            input.title = err.message;
            input.style.borderColor = '#a84a56';
            alert('Invalid auto function Y.');
        }
    };

    window.previewAutoFunc = (axis = 'x') => {
        let o = selectedObj();
        if(!o) return;
        const isY = axis === 'y';
        const input = document.getElementById(isY ? 'p_auto_func_y' : 'p_auto_func_x');
        if(!input) return;
        const fallback = isY ? 'cos(TAU * t)' : (o.type === 'joystick' ? 'sin(TAU * t)' : 'sin01(t)');
        const next = (input.value || '').trim() || fallback;
        try {
            compileAutoExpression(next);
            input.title = '';
            input.style.borderColor = '#2a5a72';
            if(isY) o.autoFuncY = next;
            else o.autoFuncX = next;
            renderAutoFunctionPreview(o);
        } catch(err) {
            input.title = err.message;
            input.style.borderColor = '#a84a56';
            renderAutoFunctionPreview(o, `${isY ? 'Y(t)' : getAutoPrimaryFunctionLabel(o)}: ${err.message}`);
        }
    };

    window.updateAutoSpeed = () => {
        let o = selectedObj();
        if(o) {
            let v = parseFloat(document.getElementById('p_auto_speed').value);
            if(!Number.isFinite(v)) v = 0.015;
            v = clamp(v, 0, 0.2);
            document.getElementById('p_auto_speed').value = v;
            o.autoSpeed = v;
        }
    };

    window.updateAutoResponse = () => {
        let o = selectedObj();
        if(o) {
            let v = parseFloat(document.getElementById('p_auto_response').value);
            if(!Number.isFinite(v)) v = 0.22;
            v = clamp(v, 0.01, 1);
            document.getElementById('p_auto_response').value = v;
            o.autoResponse = v;
        }
    };

    window.updateAutoBounce = () => {
        let o = selectedObj();
        if(o) {
            let v = parseFloat(document.getElementById('p_auto_bounce').value);
            if(!Number.isFinite(v)) v = 0.25;
            v = clamp(v, 0, 1);
            document.getElementById('p_auto_bounce').value = v;
            o.autoBounce = v;
        }
    };
    
    window.updateGravity = () => {
        let o = selectedObj();
        if(o) {
            let v = parseFloat(document.getElementById('p_gravity').value) || 0;
            if(v < 0) v = 0;
            if(v > 1) v = 1;
            document.getElementById('p_gravity').value = v;
            o.gravity = v;
        }
    };
    
    window.updateChaosRate = () => {
        let o = selectedObj();
        if(o) {
            let v = parseInt(document.getElementById('p_chaos_rate').value) || 96;
            if(v < 1) v = 1;
            if(v > 240) v = 240;
            document.getElementById('p_chaos_rate').value = v;
            o.chaosRate = v;
            renderAutoFunctionPreview(o);
        }
    };
    
    window.updateVar = (i,v) => {
        const o = selectedObj();
        if(!o) return;
        // 用于 toggle/grid 模式：直接设单个变量名
        o.vars[i] = sanitizeIniVarToken(v, '');
        renderAll();
    };
    // 逐行添加模式：更新某个槽中的某一行变量
    window.updateVarRow = (slotIdx, rowIdx, v) => {
        const o = selectedObj();
        if(!o) return;
        const raw = String(o.vars[slotIdx] || '');
        const parts = raw.split(',').map(s => s.trim());
        const cleaned = sanitizeIniVarToken(v, '');
        if(rowIdx < parts.length) parts[rowIdx] = cleaned;
        else parts.push(cleaned);
        o.vars[slotIdx] = parts.join(', ');
        renderAll();
    };
    // 逐行添加模式：为某个槽添加一行
    window.addVarRow = (slotIdx) => {
        const o = selectedObj();
        if(!o) return;
        const raw = String(o.vars[slotIdx] || '');
        // 直接追加逗号+空行
        o.vars[slotIdx] = raw ? raw + ', ' : ', ';
        renderVarInputs(o);
    };
    // 逐行添加模式：删除某个槽的某一行
    window.removeVarRow = (slotIdx, rowIdx) => {
        const o = selectedObj();
        if(!o) return;
        const raw = String(o.vars[slotIdx] || '');
        const parts = raw.split(',').map(s => s.trim());
        if(parts.length <= 1) {
            o.vars[slotIdx] = '';
        } else {
            parts.splice(rowIdx, 1);
            o.vars[slotIdx] = parts.join(', ');
        }
        renderVarInputs(o);
    };
    window.updatePath = (k,v,input) => {
        const o = selectedObj();
        if(!o) return;
        if(!o.paths) o.paths = {};
        const normalizedPath = normalizeAssetPath(v);
        o.paths[k] = normalizedPath;
        if(input && input.value !== normalizedPath) input.value = normalizedPath;
    };
    window.updateResourceOpacity = (k, v) => {
        const o = selectedObj();
        if(!o) return;
        ensureComponentResourceOpacityState(o);
        o.resourceOpacity[k] = normalizeResourceOpacityValue(o, k, v);
        refreshResourceWindow(o);
        renderAll();
    };
    
    async function assignTextureFile(obj, key, file) {
        if(!obj || !key || !isSupportedTextureFile(file)) return false;
        const dataUrl = await readFileAsDataURL(file);
        if(!obj.paths) obj.paths = {};
        if(!obj.preview) obj.preview = {};
        if(!obj.embeddedAssets) obj.embeddedAssets = {};
        const normalizedName = normalizeAssetPath(file.name);
        obj.preview[key] = dataUrl;
        obj.paths[key] = normalizedName;
        obj.embeddedAssets[key] = { dataUrl, sourceName: normalizedName };
        markHistoryDirty();
        refreshResourceWindow(obj);
        renderAll();
        return true;
    }

    async function dropTexture(event, key) {
        const file = getDroppedTextureFile(event);
        const obj = selectedObj();
        if(!file || !obj) return;
        try {
            await assignTextureFile(obj, key, file);
        } catch(err) {
            console.error(err);
            alert('读取资源文件失败。');
        }
    }

    window.uploadTex = async (k,i) => {
        const file = i.files[0];
        const obj = selectedObj();
        if(!file || !obj) return;
        try {
            await assignTextureFile(obj, k, file);
        } catch(err) {
            console.error(err);
            alert('读取资源文件失败。');
        }
        i.value = '';
    };
    
    window.deleteItem = () => {
        markHistoryDirty();
        if(!selectedEntity) return;
        if(selectedEntity.type === 'group') {
            const group = selectedGroup();
            if(!group) return;
            const removedDialogueIds = new Set(dialogueLogic.dialogues.filter(item => item.groupId === group.id).map(item => item.id));
            dialogueLogic.dialogues = dialogueLogic.dialogues.filter(item => item.groupId !== group.id);
            dialogueLogic.main.nodes = dialogueLogic.main.nodes.filter(node => !(node.type === 'dialogue' && node.config && node.config.groupId === group.id));
            const mainIds = new Set(dialogueLogic.main.nodes.map(node => node.id));
            dialogueLogic.main.edges = dialogueLogic.main.edges.filter(edge => mainIds.has(edge.fromNodeId) && mainIds.has(edge.toNodeId));
            if(removedDialogueIds.has(blueprintScopeId)) blueprintScopeId = 'main';
            removeChildFromParent(makeEntityRef('group', group.id));
            groups = groups.filter(item => item.id !== group.id);
        } else if(selectedEntity.type === 'component') {
            const componentId = selectedEntity.id;
            [dialogueLogic.main, ...dialogueLogic.dialogues].forEach(graph => {
                const removedNodeIds = new Set((graph.nodes || []).filter(node => (node.type === 'text' && node.config && node.config.componentId === componentId) || (node.type === 'random' && node.config && node.config.sourceTextId === componentId)).map(node => node.id));
                graph.nodes = (graph.nodes || []).filter(node => !removedNodeIds.has(node.id));
                graph.nodes.filter(node => node.type === 'step').forEach(node => { node.config.textIds = (node.config.textIds || []).filter(id => id !== componentId); });
                graph.nodes.filter(node => node.type === 'random').forEach(node => { node.config.branches = (node.config.branches || []).filter(branch => branch.targetTextId !== componentId); });
                graph.edges = (graph.edges || []).filter(edge => !removedNodeIds.has(edge.fromNodeId) && !removedNodeIds.has(edge.toNodeId));
            });
            removeChildFromParent(makeEntityRef('component', componentId));
            components = components.filter(component => component.id !== componentId);
            groups.forEach(group => {
                group.children = (group.children || []).filter(child => !(child.type === 'component' && child.id === componentId));
            });
        }
        ensureHierarchyIntegrity();
        clearSelection();
        propPanel.style.display='none';
        renderHierarchyPanel();
        renderAll();
    };

    async function generateAndDownloadAssets() {
        if (!window.JSZip) { alert("JSZip 未加载。"); return; }
        const zip = new JSZip();
        const fontFolder = zip.folder("font");
        const resFolder = zip.folder("res");
        const btn = document.getElementById('generateAssetsDockBtn');
        const oldText = btn ? btn.innerHTML : '';
        if(btn) {
            btn.disabled = true;
            btn.textContent = '打包中';
        }

        const fontAssets = buildFontAssetDataMap();
        const fxAssetsEnabled = getPersistentAnimSettings().enabled;
        const proceduralAssets = { ...buildProceduralAssetDataMap() };
        if(!fxAssetsEnabled) delete proceduralAssets[DEFAULT_ASSET_PATHS.fxWhite];
        const packagedAssets = collectAssetPackageEntries(components, proceduralAssets);
        // FX 白图 __fx_white.png 不挂在任何组件路径上，需在开启“常驻流光与表面高光”时显式打入，
        // 与 INI 中 [CustomShaderFx] 高光/悬停光引用保持一致；关闭时 INI 不引用、包内也不含。
        if(fxAssetsEnabled) {
            packagedAssets.set(DEFAULT_ASSET_PATHS.fxWhite, { type: 'dataUrl', source: proceduralAssets[DEFAULT_ASSET_PATHS.fxWhite], origin: DEFAULT_ASSET_PATHS.fxWhite });
        }
        // 格子滑条档位刻度引用独立白图 __grid_tick.png，与“常驻流光与表面高光”开关无关：
        // 只要存在格子模式滑条就始终打包，保证 INI 引用与资源包一致。
        if(components.some(isSliderGridMode)) {
            packagedAssets.set(DEFAULT_ASSET_PATHS.gridTick, { type: 'dataUrl', source: proceduralAssets[DEFAULT_ASSET_PATHS.gridTick], origin: DEFAULT_ASSET_PATHS.gridTick });
        }

        try {
            if(shaderTemplateSource && shaderTemplateSource.trim()) {
                resFolder.file(shaderTemplateFileName || 'draw_2d.hlsl', shaderTemplateSource);
            }
            if(fxAssetsEnabled && shaderFxSource && shaderFxSource.trim()) {
                resFolder.file(shaderFxFileName || 'draw_2d_fx.hlsl', shaderFxSource);
            }
            for (const [fileName, dataUrl] of Object.entries(fontAssets)) {
                const blob = await fetch(dataUrl).then(r => r.blob());
                fontFolder.file(fileName, blob);
            }
            const failedAssets = [];
            for (const [zipPath, entry] of packagedAssets.entries()) {
                try {
                    const sourceUrl = entry.type === 'external' ? resolveAssetFetchUrl(entry.source) : entry.source;
                    if(!sourceUrl) {
                        failedAssets.push(entry.origin || zipPath);
                        continue;
                    }
                    const response = await fetch(sourceUrl);
                    if(!response.ok) throw new Error(`HTTP ${response.status}`);
                    const blob = await response.blob();
                    resFolder.file(zipPath, blob);
                } catch (err) {
                    console.warn('资源打包失败:', entry.origin || zipPath, err);
                    failedAssets.push(entry.origin || zipPath);
                }
            }
            if(failedAssets.length > 0) {
                throw new Error(`以下资源无法正确打包，请检查路径或来源：\n${failedAssets.join('\n')}`);
            }
            const content = await zip.generateAsync({type: "blob"});
            if (ssmtHostBridge.available()) {
                const buffer = await content.arrayBuffer();
                const result = await ssmtHostBridge.request('save-assets', { buffer: buffer });
                alert('资源包已保存到当前工作空间：\n' + result.path);
            } else {
                const link = document.createElement('a');
                link.href = URL.createObjectURL(content);
                link.download = 'ui_assets_' + Date.now() + '.zip';
                link.click();
            }
        } catch(e) { alert("Error: " + e.message); } 
        finally {
            if(btn) {
                btn.disabled = false;
                btn.innerHTML = oldText;
            }
        }
    }

    function renderAll() {
        try {
        if(!root.isConnected || !workArea?.isConnected) return;
        invalidateLinkedValuesCache();
        flushHistorySnapshot('render');
        syncPreviewClock();
        workArea.innerHTML = '';
        const area = getWorkAreaPixelSize();
        const previewSnapshot = getPreviewSimulationSnapshot();
        const persistentSheenState = getPersistentPreviewSheenState();

        const rootGroups = getRootRefs().filter(ref => ref.type === 'group');
        const renderGroupOverlay = (groupRef) => {
            const group = getGroupById(groupRef.id);
            if(!group) return;
            const bounds = getGroupNormalizedBounds(group.id);
            if(!bounds || !Number.isFinite(bounds.w) || !Number.isFinite(bounds.h)) return;
            const overlay = mkDiv(group.id, 'group-node', bounds);
            overlay.style.zIndex = 1;
            const isSelected = selectedEntities.some(ref => isSameEntityRef(ref, groupRef));
            if(isSelected) overlay.classList.add(selectedEntity && isSameEntityRef(selectedEntity, groupRef) ? 'selected' : 'selected-multi');
            overlay.onmousedown = (e) => {
                if(e.target === badge) return;
                startDrag(e, groupRef, overlay);
            };
            overlay.onclick = (e) => {
                if(e.target === badge) return;
                e.stopPropagation();
                if(dragSession && dragSession.targetKey === entityRefKey(groupRef) && dragSession.suppressClick) {
                    dragSession = null;
                    return;
                }
                selectEntity(groupRef, e.ctrlKey);
                dragSession = null;
            };
            const badge = document.createElement('button');
            badge.type = 'button';
            badge.className = 'group-node-badge';
            badge.innerHTML = `<span>组</span><strong>${escapeHtml(sanitizeGroupDisplayName(group.name, group.id))}</strong>`;
            badge.onmousedown = (e) => startDrag(e, groupRef, overlay);
            badge.onclick = (e) => {
                e.stopPropagation();
                if(dragSession && dragSession.targetKey === entityRefKey(groupRef) && dragSession.suppressClick) {
                    dragSession = null;
                    return;
                }
                selectEntity(groupRef, e.ctrlKey);
                dragSession = null;
            };
            overlay.appendChild(badge);
            workArea.appendChild(overlay);
            (group.children || []).filter(child => child.type === 'group').forEach(renderGroupOverlay);
        };
        rootGroups.forEach(renderGroupOverlay);

        components.forEach(mod => {
            if(mod.type === 'text' && !getPreviewTextVisible(mod)) return;
            const rect = getComponentPixelRect(mod);
            const handlePx = getComponentHandleMetrics(mod);
            const trackPx = getTrackPixelThickness(mod);
            const globalAnimState = getGlobalPreviewAnimState(mod);
            const localAnimState = getLocalPreviewAnimState(mod);
            const previewNodeAnim = getPreviewNodeAnimState(mod, localAnimState);
            const tex = {
                bg: getPreviewTextureUrl(mod, 'bg'),
                img: getPreviewTextureUrl(mod, 'img'),
                handle: getPreviewTextureUrl(mod, 'handle'),
                post: getPreviewTextureUrl(mod, 'post') || getPreviewTextureUrl(mod, 'post_marker'),
                bar_l: getPreviewTextureUrl(mod, 'bar_l'),
                bar_r: getPreviewTextureUrl(mod, 'bar_r'),
                off: getPreviewTextureUrl(mod, 'off'),
                on: getPreviewTextureUrl(mod, 'on'),
                prog_off: getPreviewTextureUrl(mod, 'prog_off'),
                prog_on: getPreviewTextureUrl(mod, 'prog_on')
            };

            let mDiv = mkDiv(mod.id, 'node mod-node', mod);
            if((mod.type === 'static' || mod.type === 'sequence') && mod.followCursor === true && workspaceMode === 'run') {
                const followFraction = getFollowCursorOffsets(mod);
                mDiv.style.left = ((previewPointerState.x - mod.w * followFraction.x) * 100) + '%';
                mDiv.style.top = ((previewPointerState.y - mod.h * followFraction.y) * 100) + '%';
                mDiv.style.pointerEvents = 'none';
            }
            mDiv.style.zIndex = mod.zIndex;
            mDiv.style.borderRadius = `${getComponentCornerRadiusPx(mod)}px`;
            const nodeTx = globalAnimState.translateX + (previewNodeAnim.offsetX || 0);
            const nodeTy = globalAnimState.translateY + (previewNodeAnim.offsetY || 0);
            mDiv.style.transform = `translate(${nodeTx.toFixed(2)}px, ${nodeTy.toFixed(2)}px) rotate(${((mod.rot||0) + (previewNodeAnim.rotate || 0)).toFixed(3)}deg) scale(${(globalAnimState.scale * previewNodeAnim.scale).toFixed(4)}) scale(var(--text-hover-scale, 1))`;
            mDiv.style.opacity = (previewNodeAnim.opacity || 1).toFixed(3);
            mDiv.style.setProperty('--node-accent', ({
                slider_h: 'rgba(143,232,255,0.30)',
                slider_v: 'rgba(143,232,255,0.30)',
                joystick: 'rgba(93,242,193,0.26)',
                toggle: 'rgba(255,190,92,0.28)',
                accum: 'rgba(255,90,90,0.28)',
                static: 'rgba(255,255,255,0.16)',
                sequence: 'rgba(142, 68, 173, 0.28)',
                text: 'rgba(255,142,92,0.22)'
            }[mod.type] || 'rgba(143,232,255,0.26)'));
            if(mod.type === 'sequence' || mod.type === 'toggle') mDiv.setAttribute('data-animated', '1');
            if(selectedEntity && selectedEntity.type === 'component' && selectedEntity.id === mod.id) mDiv.classList.add('selected');
            else if(selectedEntities.some(ref => ref.type === 'component' && ref.id === mod.id)) mDiv.classList.add('selected-multi');
            if(getOwningGroupForComponent(mod)) mDiv.classList.add('grouped');
            if(getEffectiveGroupPinned(mod)) mDiv.classList.add('pinned');

            const persistentSurfaceStyle = persistentSheenState.enabled ? '' : ' style="display:none;"';
            let html = `<div class="node-shell"${persistentSurfaceStyle}></div><div class="node-sheen"></div><div class="node-content">`;
            const hasLocalFlow = persistentSheenState.enabled && (localAnimState.mode === 'shimmer' || localAnimState.mode === 'sheen' || localAnimState.mode === 'radial_sheen');
            const sheenOffset = hasLocalFlow ? localAnimState.sheenOffset : persistentSheenState.offsetX;
            const sheenOpacityBase = hasLocalFlow
                ? Math.min(0.58, 0.08 + (localAnimState.fxBoost || 0) * 0.34)
                : persistentSheenState.opacity;
            const sheenOpacity = sheenOpacityBase * getComponentResourceOpacity(mod, 'bg');
            const sheenStyle = ` style="transform:translateX(${sheenOffset.toFixed(4)}px) rotate(10deg); opacity:${sheenOpacity.toFixed(3)};"`;
            html = `<div class="node-shell"${persistentSurfaceStyle}></div><div class="node-sheen"${sheenStyle}></div><div class="node-content">`;
            if(tex.bg) html += `<div class="component-bg" style="background-image:url(${tex.bg}); opacity:${getPreviewBackgroundOpacity(mod).toFixed(3)};"></div>`;

            if(mod.type === 'static') {
                html += `<div class="vis-label">Img (Z:${mod.zIndex})</div>`;
                if(tex.img) html += `<div class="component-surface" style="inset:0; border-radius:inherit; background-image:url(${tex.img}); opacity:${getComponentResourceOpacity(mod, 'img').toFixed(3)};"></div>`;
                else html += `<div class="component-surface placeholder" style="inset:0; border-radius:inherit;">无图片</div>`;
            } else if(mod.type === 'sequence') {
                html += `<div class="vis-label">Seq (Z:${mod.zIndex})</div>`;
                const previewFrame = getSequencePreviewFrame(mod, previewSnapshot);
                if(previewFrame && previewFrame.preview) html += `<div class="component-surface" style="inset:0; border-radius:inherit; background-image:url(${previewFrame.preview});"></div>`;
                else html += `<div class="component-surface placeholder" style="inset:0; border-radius:inherit;">无序列帧</div>`;
            } else if(mod.type === 'text') {
                html += `<div class="vis-label">Text (Z:${mod.zIndex})</div>`;
                mDiv.classList.add('text-mode');
                if(mod.textHoverEffect) mDiv.classList.add('text-hover-enabled');
                const textLayout = buildRenderedTextLayout(mod, { valueReplacement: getTextPreviewReplacement(mod, previewSnapshot) });
                html += `<div class="text-layout" style="font-family:${escapeHtml(mod.fontFamily || 'Microsoft YaHei')}; font-weight:${mod.fontBold ? 'bold' : 'normal'}; font-style:${mod.fontItalic ? 'italic' : 'normal'};">`;
                if(textLayout.layout.length > 0) {
                    const waveStrength = localAnimState.mode === 'glyph_wave' ? (localAnimState.textWave || 0) : 0;
                    const glowStrength = localAnimState.mode === 'glyph_glow'
                        ? clamp((localAnimState.fxBoost || 0) / 1.2, 0, 1)
                        : 0;
                    textLayout.layout.forEach((slot, slotIndex) => {
                        const content = slot.char === ' ' ? '&nbsp;' : escapeHtml(slot.char);
                        const waveFactor = Math.sin(((slotIndex / Math.max(textLayout.layout.length, 1)) * Math.PI * 2) + slot.index * 0.08);
                        const topPx = slot.topPx + (waveStrength * waveFactor);
                        const charOpacity = localAnimState.mode === 'glyph_glow'
                            ? (localAnimState.opacity || 1)
                            : 1;
                        const glowBlur = localAnimState.mode === 'glyph_glow' ? (2 + glowStrength * 8) : 0;
                        const textShadow = localAnimState.mode === 'glyph_glow'
                            ? `0 0 ${glowBlur.toFixed(2)}px ${slot.color}, 0 0 ${(glowBlur * 1.8).toFixed(2)}px ${slot.color}`
                            : 'none';
                        html += `<div class="char-item" style="left:${slot.leftPx}px; top:${topPx.toFixed(2)}px; width:${slot.widthPx}px; height:${slot.heightPx}px; color:${slot.color}; font-size:${slot.fontSizePx}px; opacity:${charOpacity.toFixed(3)}; text-shadow:${textShadow};">${content}</div>`;
                    });
                } else {
                    html += `<span style="color:var(--fixed-dark-muted);font-size:10px;padding:5px;">空文本</span>`;
                }
                html += `</div>`;
            } else {
                let l = mod.vars[0] || '..';
                if(mod.type === 'joystick' && mod.paramMode === '4') {
                    const joyCfg = getJoystickConfig(mod);
                    l = `Joystick ${joyCfg.directionCount} dirs x ${joyCfg.subdivisions} seg`;
                }
                if(mod.type === 'accum') {
                    const accumThreshold = Math.max(0.0001, Number(mod.accumThreshold) || 5);
                    l = `积蓄条 ${formatPreviewVariableValue(getPreviewAccumCount(mod))}/${formatPreviewVariableValue(accumThreshold)}`;
                }
                if(mod.type === 'toggle' && isToggleMultiMode(mod)) {
                    const steps = mod.toggleSteps || DEFAULT_TOGGLE_STEPS;
                    const active = clamp(Math.round(Number(mod.initialValue) || 0), 0, steps);
                    l += ` [${active}/${steps}]`;
                }
                if(mod.type === 'toggle' && mod.toggleInvert === true) l += ' [反]';
                if(mod.switchGroup > 0) l += ` [G:${mod.switchGroup}]`;
                if(mod.physics) l += ' (P)';
                if(mod.type.includes('slider') && mod.paramMode === '3') l += ` [Grd:${mod.gridSteps||3}]`;
                if(isSliderSubdivisionMode(mod)) l += ` [Seg:${getSliderSubdivisionConfig(mod).subdivisions}]`;
                html += `<div class="vis-label">${l}</div>`;

                if(mod.type.includes('slider')) {
                    const isV = mod.type === 'slider_v';
                    const linkedOverride = getLinkedOverrideValue(mod);
                    const rawValue = getSliderPreviewValue(mod, previewSnapshot);
                    // 拖拽中或非联动目标 → 使用原始值
                    const useLinked = typeof linkedOverride === 'number' && !mod.__linkedDragging;
                    const baseValue = useLinked ? linkedOverride : rawValue;
                    // 平滑联动值，产生回弹效果
                    const currentValue = useLinked ? getLinkedSmoothedValue(mod, baseValue) : rawValue;
                    const fillPct = Math.round(currentValue * 10000) / 100;
                    const actualValue = sliderNormalizedToActual(mod, currentValue);
                    const trackTravelX = Math.max(0, rect.width - handlePx.width);
                    const trackTravelY = Math.max(0, rect.height - handlePx.height);
                    const handleLeftPx = isV ? Math.round((rect.width - handlePx.width) * 0.5) : Math.round(trackTravelX * currentValue);
                    const handleTopPx = isV ? Math.round(trackTravelY * (1 - currentValue)) : Math.round((rect.height - handlePx.height) * 0.5);
                    const localHandleLeft = handleLeftPx + Math.round(localAnimState.offsetX || 0);
                    const localHandleTop = handleTopPx + Math.round(localAnimState.offsetY || 0);
                    const handleScale = localAnimState.handleScale || 1;
                    const handleOpacity = localAnimState.handleOpacity || 1;
                    const fillOpacity = localAnimState.fillOpacity || 1;
                    const handleOffset = isV
                        ? `left:${localHandleLeft}px; top:${localHandleTop}px; transform:scale(${handleScale.toFixed(4)});`
                        : `left:${localHandleLeft}px; top:${localHandleTop}px; transform:scale(${handleScale.toFixed(4)});`;
                    const trackStyle = isV
                        ? `left:50%; top:0; transform:translateX(-50%); width:${trackPx}px; height:100%;`
                        : `left:0; top:50%; transform:translateY(-50%); width:100%; height:${trackPx}px;`;

                    html += `<div style="position:relative; width:100%; height:100%;">`;
                    html += tex.bar_r
                        ? `<div class="component-surface" style="${trackStyle} border-radius:999px; background-image:url(${tex.bar_r}); opacity:${getComponentResourceOpacity(mod, 'bar_r').toFixed(3)};"></div>`
                        : `<div class="track" style="${trackStyle}"></div>`;

                    if(mod.paramMode === '3') {
                        const gridSteps = Math.max(2, mod.gridSteps || 3);
                        for(let s = 0; s < gridSteps; s++) {
                            const pos = (s / (gridSteps - 1)) * 100;
                            html += !isV
                                ? `<div style="position:absolute; left:${pos}%; top:50%; transform:translate(-50%, -50%); width:1px; height:${Math.max(trackPx + 6, 10)}px; background:rgba(255,148,64,0.7); z-index:3;"></div>`
                                : `<div style="position:absolute; top:${100 - pos}%; left:50%; transform:translate(-50%, -50%); height:1px; width:${Math.max(trackPx + 6, 10)}px; background:rgba(255,148,64,0.7); z-index:3;"></div>`;
                        }
                    }

                    if(isSliderSubdivisionMode(mod)) {
                        const sliderCfg = getSliderSubdivisionConfig(mod);
                        const tickPositions = [];
                        if(mod.paramMode === '2') {
                            tickPositions.push(50);
                            for(let s = 1; s < sliderCfg.subdivisions; s++) {
                                const offset = (s / sliderCfg.subdivisions) * 50;
                                tickPositions.push(50 - offset, 50 + offset);
                            }
                        } else {
                            for(let s = 1; s < sliderCfg.subdivisions; s++) tickPositions.push((s / sliderCfg.subdivisions) * 100);
                        }
                        tickPositions.sort((a, b) => a - b).forEach(pos => {
                            html += !isV
                                ? `<div class="slider-subdivision-tick" style="position:absolute; left:${pos.toFixed(3)}%; top:50%; transform:translate(-50%, -50%); width:1px; height:${Math.max(trackPx + 6, 10)}px; background:rgba(143,232,255,0.78); z-index:3; pointer-events:none;"></div>`
                                : `<div class="slider-subdivision-tick" style="position:absolute; top:${(100 - pos).toFixed(3)}%; left:50%; transform:translate(-50%, -50%); height:1px; width:${Math.max(trackPx + 6, 10)}px; background:rgba(143,232,255,0.78); z-index:3; pointer-events:none;"></div>`;
                        });
                    }

                    if(mod.paramMode === '2') {
                        const leftPct = Math.max(0, (0.5 - currentValue) * 100);
                        const rightPct = Math.max(0, (currentValue - 0.5) * 100);
                        const leftStyle = isV
                            ? `left:50%; top:50%; transform:translateX(-50%); width:${trackPx}px; height:${leftPct.toFixed(2)}%;`
                            : `left:${(50 - leftPct).toFixed(2)}%; top:50%; transform:translateY(-50%); width:${leftPct.toFixed(2)}%; height:${trackPx}px;`;
                        const rightStyle = isV
                            ? `left:50%; top:${(50 - rightPct).toFixed(2)}%; transform:translateX(-50%); width:${trackPx}px; height:${rightPct.toFixed(2)}%;`
                            : `left:50%; top:50%; transform:translateY(-50%); width:${rightPct.toFixed(2)}%; height:${trackPx}px;`;
                        if(leftPct > 0.01) html += tex.bar_l ? `<div class="component-surface" style="${leftStyle} border-radius:999px; background-image:url(${tex.bar_l}); z-index:2; opacity:${(fillOpacity * getComponentResourceOpacity(mod, 'bar_l')).toFixed(3)};"></div>` : `<div class="bar-fill" style="${leftStyle}; opacity:${fillOpacity.toFixed(3)};"></div>`;
                        if(rightPct > 0.01) html += tex.bar_l ? `<div class="component-surface" style="${rightStyle} border-radius:999px; background-image:url(${tex.bar_l}); z-index:2; opacity:${(fillOpacity * getComponentResourceOpacity(mod, 'bar_l')).toFixed(3)};"></div>` : `<div class="bar-fill" style="${rightStyle}; opacity:${fillOpacity.toFixed(3)};"></div>`;
                    } else {
                        const fillStyle = isV
                            ? `left:50%; bottom:0; transform:translateX(-50%); width:${trackPx}px; height:${fillPct.toFixed(2)}%;`
                            : `left:0; top:50%; transform:translateY(-50%); width:${fillPct.toFixed(2)}%; height:${trackPx}px;`;
                        html += tex.bar_l
                            ? `<div class="component-surface" style="${fillStyle} border-radius:999px; background-image:url(${tex.bar_l}); z-index:2; opacity:${(fillOpacity * getComponentResourceOpacity(mod, 'bar_l')).toFixed(3)};"></div>`
                            : `<div class="bar-fill" style="${fillStyle}; opacity:${fillOpacity.toFixed(3)};"></div>`;
                    }

                    html += tex.handle
                        ? `<div id="handle_${mod.id}" class="component-surface" style="${handleOffset} width:${handlePx.width}px; height:${handlePx.height}px; border-radius:${handlePx.radius}px; background-image:url(${tex.handle}); z-index:4; opacity:${(handleOpacity * getComponentResourceOpacity(mod, 'handle')).toFixed(3)};"></div>`
                        : `<div id="handle_${mod.id}" class="handle" style="${handleOffset} width:${handlePx.width}px; height:${handlePx.height}px; z-index:4; opacity:${handleOpacity.toFixed(3)};"></div>`;
                    // 手柄拦截层：内联 onmousedown 直接调用全局函数，绝不进入 mDiv 事件链
                    html += `<div style="position:absolute; left:${localHandleLeft}px; top:${localHandleTop}px; width:${handlePx.width}px; height:${handlePx.height}px; border-radius:${handlePx.radius}px; z-index:20; cursor:grab; background:transparent;" onmousedown="UIB._svCapture(event,'${mod.id}')"></div>`;
                    if(sliderUsesExplicitRange(mod)) {
                        const valueLabel = Number.isInteger(actualValue)
                            ? String(actualValue)
                            : actualValue.toFixed(3).replace(/0+$/, '').replace(/\.$/, '');
                        html += `<div style="position:absolute; right:6px; top:6px; z-index:22; font-size:10px; line-height:1; color:#eaf7ff; background:rgba(5,16,28,0.72); border:1px solid rgba(126,211,252,0.28); border-radius:999px; padding:3px 6px; pointer-events:none;">${valueLabel}</div>`;
                    }
                    html += `</div>`;
                } else if(mod.type === 'joystick') {
                    const joyCfg = mod.paramMode === '4' ? getJoystickConfig(mod) : null;
                    const joystickOverride = getLinkedOverrideValue(mod);
                    const rawJoyVector = getJoystickPreviewVector(mod, previewSnapshot);
                    let joyVector;
                    if(joystickOverride && typeof joystickOverride === 'object' && !mod.__linkedDragging) {
                        // 使用弹簧平滑的联动值（产生回弹）
                        const smoothedX = getLinkedSmoothedValue(mod, typeof joystickOverride.x === 'number' ? joystickOverride.x : rawJoyVector.x, 'x');
                        const smoothedY = getLinkedSmoothedValue(mod, typeof joystickOverride.y === 'number' ? joystickOverride.y : rawJoyVector.y, 'y');
                        joyVector = clampJoystickVectorToRoundedBounds(mod, smoothedX, smoothedY);
                    } else {
                        joyVector = clampJoystickVectorToRoundedBounds(mod, rawJoyVector.x, rawJoyVector.y);
                    }
                    const joyTravelX = Math.max(0, rect.width - handlePx.width);
                    const joyTravelY = Math.max(0, rect.height - handlePx.height);
                    const handleLeftPx = Math.round((joyTravelX * 0.5) + joyVector.x * (joyTravelX * 0.5) + (localAnimState.offsetX || 0));
                    const handleTopPx = Math.round((joyTravelY * 0.5) - joyVector.y * (joyTravelY * 0.5) + (localAnimState.offsetY || 0));
                    const handleScale = localAnimState.handleScale || 1;
                    const handleOpacity = localAnimState.handleOpacity || 1;
                    html += `<div style="position:relative; width:100%; height:100%;">`;
                    if(persistentSheenState.enabled && localAnimState.mode === 'radial_sheen') {
                        const radialOpacity = Math.min(0.72, 0.12 + (localAnimState.fxBoost || 0) * 0.22);
                        const radialRotate = localAnimState.phase01 * 360;
                        if(tex.bar_r) {
                            html += `<div class="component-surface" style="left:50%; top:50%; transform:translate(-50%, -50%) rotate(${radialRotate.toFixed(2)}deg); width:${Math.max(10, rect.width - trackPx)}px; height:${Math.max(10, rect.height - trackPx)}px; border-radius:50%; background-image:url(${tex.bar_r}); opacity:${(radialOpacity * getComponentResourceOpacity(mod, 'bar_r')).toFixed(3)};"></div>`;
                        } else {
                            html += `<div style="position:absolute; left:50%; top:50%; width:${Math.max(10, rect.width - trackPx)}px; height:${Math.max(10, rect.height - trackPx)}px; transform:translate(-50%, -50%) rotate(${radialRotate.toFixed(2)}deg); border-radius:50%; box-shadow:0 0 0 2px rgba(111,223,255,${(0.28 + (localAnimState.fxBoost || 0) * 0.16).toFixed(3)}) inset, 0 0 16px rgba(111,223,255,${(0.08 + (localAnimState.fxBoost || 0) * 0.08).toFixed(3)});"></div>`;
                        }
                    }
                    if(joyCfg) {
                        for(let ring = 1; ring <= joyCfg.subdivisions; ring++) {
                            const size = (ring / joyCfg.subdivisions) * 100;
                            html += `<div style="position:absolute; left:50%; top:50%; width:${size}%; height:${size}%; transform:translate(-50%, -50%); border:1px dashed rgba(126,211,252,${ring === joyCfg.subdivisions ? 0.46 : 0.24}); border-radius:50%; box-sizing:border-box; pointer-events:none;"></div>`;
                        }
                        for(let dirIdx = 0; dirIdx < joyCfg.directionCount; dirIdx++) {
                            const angle = getJoystickDirectionAngle(mod, dirIdx);
                            const angleRad = angle * Math.PI / 180;
                            const labelRadius = 0.44;
                            const labelX = (50 + Math.sin(angleRad) * (labelRadius * 100)).toFixed(2);
                            const labelY = (50 - Math.cos(angleRad) * (labelRadius * 100)).toFixed(2);
                            html += `<div style="position:absolute; left:50%; top:50%; width:1px; height:42%; background:linear-gradient(180deg, rgba(126,211,252,0.72), rgba(126,211,252,0.14)); transform-origin:center top; transform:translateX(-50%) rotate(${angle}deg); pointer-events:none;"></div>`;
                            html += `<div style="position:absolute; left:${labelX}%; top:${labelY}%; transform:translate(-50%, -50%); font-size:10px; line-height:1; color:#d8ecf7; background:rgba(7,15,24,0.72); border:1px solid rgba(126,211,252,0.34); border-radius:999px; padding:2px 5px; white-space:nowrap; box-shadow:0 0 10px rgba(0,0,0,0.28); pointer-events:none;">${formatAngleLabel(angle)}</div>`;
                        }
                    }
                    html += tex.handle
                        ? `<div class="component-surface" style="left:${handleLeftPx}px; top:${handleTopPx}px; width:${handlePx.width}px; height:${handlePx.height}px; border-radius:${handlePx.radius}px; background-image:url(${tex.handle}); z-index:4; transform:scale(${handleScale.toFixed(4)}); opacity:${(handleOpacity * getComponentResourceOpacity(mod, 'handle')).toFixed(3)};"></div>`
                        : `<div class="handle" style="left:${handleLeftPx}px; top:${handleTopPx}px; width:${handlePx.width}px; height:${handlePx.height}px; z-index:4; transform:scale(${handleScale.toFixed(4)}); opacity:${handleOpacity.toFixed(3)};"></div>`;
                    html += `<div style="position:absolute; left:${handleLeftPx}px; top:${handleTopPx}px; width:${handlePx.width}px; height:${handlePx.height}px; border-radius:${handlePx.radius}px; z-index:20; cursor:grab; background:transparent;" onmousedown="UIB._svCapture(event,'${mod.id}')"></div>`;
                    // 碰撞桩红点：显示在源组件上，每个联动目标一个碰撞桩
                    const collisionPosts = getJoystickCollisionPosts(mod);
                    if(collisionPosts.length > 0) {
                        const colors = ['#ff2222', '#22ddff', '#ffaa22', '#22ff66', '#ff44dd', '#ffff22'];
                        const postCenterX = rect.width * 0.5;
                        const postCenterY = rect.height * 0.5;
                        collisionPosts.forEach((post, idx) => {
                            const postLeftPx = Math.round(postCenterX + post.posX * (joyTravelX * 0.5));
                            const postTopPx = Math.round(postCenterY - post.posY * (joyTravelY * 0.5));
                            const postR = Math.round(Math.max(4, post.radius * Math.min(joyTravelX, joyTravelY) * 0.28));
                            const color = colors[idx % colors.length];
                            const labelTextColor = getReadableTextColor(color);
                            const shadowColor = color === '#ff2222' ? 'rgba(255,40,40,0.9)' : (color + 'cc');
                            if(tex.post) {
                                html += `<div class="component-surface" style="left:${postLeftPx - postR}px; top:${postTopPx - postR}px; width:${postR*2}px; height:${postR*2}px; border-radius:50%; background-image:url(${tex.post}); z-index:26; pointer-events:none; opacity:${getComponentResourceOpacity(mod, 'post').toFixed(3)}; box-shadow:0 0 14px ${shadowColor};"></div>`;
                            } else {
                                html += `<div style="position:absolute; left:${postLeftPx}px; top:${postTopPx}px; width:${postR*2}px; height:${postR*2}px; transform:translate(-50%,-50%); background:${color}; border:2px solid #fff; border-radius:50%; z-index:26; pointer-events:none; box-shadow:0 0 14px ${shadowColor};"></div>`;
                            }
                            html += `<div style="position:absolute; left:${postLeftPx}px; top:${postTopPx - postR - 2}px; transform:translate(-50%,-100%); font-size:9px; color:${labelTextColor}; background:${color}; border-radius:3px; padding:1px 4px; z-index:26; pointer-events:none; white-space:nowrap;">P${idx+1}</div>`;
                        });
                    }
                    // 四边形区域可视化：绘制联动映射区域边界和碰撞桩
                    try {
                    const linkedRegions = getJoystickLinkedRegions(mod);
                    const triggerRegions = getJoystickRangeTriggerRegions(mod);
                    if(linkedRegions.length > 0 || triggerRegions.length > 0) {
                        const regionColors = ['rgba(255,100,50,', 'rgba(50,200,255,', 'rgba(255,200,50,', 'rgba(50,255,100,', 'rgba(255,50,200,', 'rgba(200,200,50,'];
                        const cornerColors = ['#ff6432', '#32c8ff', '#ffc832', '#32ff64', '#ff32c8', '#c8c832'];
                        const triggerRegionColors = ['rgba(63,217,255,', 'rgba(86,225,193,', 'rgba(120,168,255,', 'rgba(100,220,255,'];
                        const triggerCornerColors = ['#3fd9ff', '#56e1c1', '#78a8ff', '#64dcff'];
                        const centerX = rect.width * 0.5;
                        const centerY = rect.height * 0.5;
                        const halfTravelX = joyTravelX * 0.5;
                        const halfTravelY = joyTravelY * 0.5;
                        let svgContent = '';
                        linkedRegions.forEach((region, idx) => {
                            const color = regionColors[idx % regionColors.length];
                            const pts = region.points;
                            const pointsStr = pts.map(p => {
                                const sx = centerX + (Number.isFinite(p.x) ? p.x : 0) * halfTravelX;
                                const sy = centerY - (Number.isFinite(p.y) ? p.y : 0) * halfTravelY;
                                return `${sx.toFixed(1)},${sy.toFixed(1)}`;
                            }).join(' ');
                            // 填充区域 + 虚线边框（透明度0.25）
                            svgContent += `<polygon points="${pointsStr}" fill="${color}0.25)" stroke="${color}0.6)" stroke-width="1.5" stroke-dasharray="5,3" stroke-linejoin="round"/>`;
                            // 4个角点标记
                            pts.forEach((p, pi) => {
                                const sx = centerX + (Number.isFinite(p.x) ? p.x : 0) * halfTravelX;
                                const sy = centerY - (Number.isFinite(p.y) ? p.y : 0) * halfTravelY;
                                svgContent += `<circle cx="${sx.toFixed(1)}" cy="${sy.toFixed(1)}" r="3.5" fill="${cornerColors[idx % cornerColors.length]}" stroke="#fff" stroke-width="0.8"/>`;
                                svgContent += `<text x="${sx.toFixed(1)}" y="${(sy - 8).toFixed(1)}" text-anchor="middle" font-size="9" fill="${cornerColors[idx % cornerColors.length]}" font-weight="bold">${pi+1}</text>`;
                            });
                            // 碰撞桩（如果启用）
                            if(region.postEnabled) {
                                const pcx = centerX + region.postCenterX * halfTravelX;
                                const pcy = centerY - region.postCenterY * halfTravelY;
                                const pr = Math.max(6, region.postRadius * Math.min(halfTravelX, halfTravelY) * 0.56);
                                svgContent += `<circle cx="${pcx.toFixed(1)}" cy="${pcy.toFixed(1)}" r="${pr.toFixed(1)}" fill="none" stroke="rgba(255,60,60,0.7)" stroke-width="2" stroke-dasharray="4,3"/>`;
                                svgContent += `<circle cx="${pcx.toFixed(1)}" cy="${pcy.toFixed(1)}" r="2.5" fill="#ff4040" stroke="#fff" stroke-width="0.5"/>`;
                                svgContent += `<text x="${pcx.toFixed(1)}" y="${(pcy - pr - 4).toFixed(1)}" text-anchor="middle" font-size="8" fill="#ff6060" font-weight="bold">桩${idx+1}</text>`;
                            }
                        });
                        triggerRegions.forEach((region, idx) => {
                            const color = triggerRegionColors[idx % triggerRegionColors.length];
                            const strokeColor = triggerCornerColors[idx % triggerCornerColors.length];
                            const pts = region.points;
                            const pointsStr = pts.map(p => {
                                const sx = centerX + (Number.isFinite(p.x) ? p.x : 0) * halfTravelX;
                                const sy = centerY - (Number.isFinite(p.y) ? p.y : 0) * halfTravelY;
                                return `${sx.toFixed(1)},${sy.toFixed(1)}`;
                            }).join(' ');
                            svgContent += `<polygon points="${pointsStr}" fill="${color}0.18)" stroke="rgba(255,255,255,0.95)" stroke-width="2.2" stroke-linejoin="round"/>`;
                            svgContent += `<polygon points="${pointsStr}" fill="none" stroke="${color}0.96)" stroke-width="1.4" stroke-dasharray="3,2" stroke-linejoin="round"/>`;
                            pts.forEach((p, pi) => {
                                const sx = centerX + (Number.isFinite(p.x) ? p.x : 0) * halfTravelX;
                                const sy = centerY - (Number.isFinite(p.y) ? p.y : 0) * halfTravelY;
                                svgContent += `<circle cx="${sx.toFixed(1)}" cy="${sy.toFixed(1)}" r="3.2" fill="${strokeColor}" stroke="#fff" stroke-width="0.8"/>`;
                                svgContent += `<text x="${sx.toFixed(1)}" y="${(sy + 14).toFixed(1)}" text-anchor="middle" font-size="8" fill="${strokeColor}" font-weight="bold">T${region.triggerIndex + 1}.${pi + 1}</text>`;
                            });
                        });
                        html += `<svg style="position:absolute; top:0; left:0; width:100%; height:100%; pointer-events:none; z-index:10;" viewBox="0 0 ${rect.width} ${rect.height}">${svgContent}</svg>`;
                    }
                    } catch(e) { console.error('getJoystickLinkedRegions svg render error:', e); }
                    html += `</div>`;
                } else if(mod.type === 'toggle') {
                    if(isToggleMultiMode(mod)) {
                        const steps = Math.max(1, mod.toggleSteps || DEFAULT_TOGGLE_STEPS);
                        const active = clamp(Math.round(Number(mod.initialValue) || 0), 0, steps);
                        const gapPx = Math.max(2, Math.round(Math.min(rect.width, rect.height) * 0.03));
                        const padPx = Math.max(4, Math.round(Math.min(rect.width, rect.height) * 0.05));
                        const radius = Math.max(6, Math.round(Math.min(rect.width, rect.height) * 0.08));
                        const segScale = localAnimState.mode === 'toggle_pop' ? (localAnimState.scale || 1) : 1;
                        const segShift = localAnimState.mode === 'toggle_slide' ? (localAnimState.stateOffsetX || 0) * 0.08 : 0;
                        html += `<div style="position:relative; width:100%; height:100%; padding:${padPx}px; box-sizing:border-box;">`;
                        if(tex.bg) html += `<div class="component-bg" style="background-image:url(${tex.bg}); opacity:${getPreviewBackgroundOpacity(mod).toFixed(3)};"></div>`;
                        html += `<div style="position:relative; z-index:2; display:flex; gap:${gapPx}px; align-items:stretch; width:100%; height:100%;">`;
                        for(let s = 0; s < steps; s++) {
                            const segTex = s < active ? tex.prog_on : tex.prog_off;
                            const segStyle = `position:relative; flex:1; border-radius:${radius}px; transform:translateX(${(s < active ? segShift : 0).toFixed(2)}px) scale(${segScale.toFixed(4)});`;
                            html += segTex
                                ? `<div class="component-surface" style="${segStyle} background-image:url(${segTex}); opacity:${((s < active ? (localAnimState.opacity || 1) : 1) * getComponentResourceOpacity(mod, s < active ? 'prog_on' : 'prog_off')).toFixed(3)};"></div>`
                                : `<div style="${segStyle} background:${s < active ? '#ffb54d' : '#2a5a72'}; opacity:${(s < active ? (localAnimState.opacity || 1) : 1).toFixed(3)};"></div>`;
                        }
                        html += `</div></div>`;
                    } else {
                        const isOn = Number(mod.initialValue) > 0;
                        const stateTex = isOn ? tex.on : tex.off;
                        const ghostTex = isOn ? tex.off : tex.on;
                        const stateShift = localAnimState.mode === 'toggle_slide' ? (localAnimState.stateOffsetX || 0) : 0;
                        const stateScale = localAnimState.mode === 'toggle_pop' ? (localAnimState.scale || 1) : 1;
                        if(localAnimState.mode === 'toggle_slide' && ghostTex) {
                            html += `<div class="component-surface" style="inset:0; border-radius:inherit; background-image:url(${ghostTex}); transform:translateX(${(-stateShift * 0.55).toFixed(2)}px); opacity:${(0.18 * getComponentResourceOpacity(mod, isOn ? 'off' : 'on')).toFixed(3)};"></div>`;
                        }
                        if(stateTex) html += `<div class="component-surface" style="inset:0; border-radius:inherit; background-image:url(${stateTex}); transform:translateX(${stateShift.toFixed(2)}px) scale(${stateScale.toFixed(4)}); opacity:${((localAnimState.opacity || 1) * getComponentResourceOpacity(mod, isOn ? 'on' : 'off')).toFixed(3)};"></div>`;
                       else html += `<div class="component-surface placeholder" style="inset:0; border-radius:inherit;">SW</div>`;
                   }
                } else if(mod.type === 'accum') {
                    const isV = mod.accumDirection === 'v';
                    const accumThreshold = Math.max(0.0001, Number(mod.accumThreshold) || 5);
                    const accumCount = Math.max(0, getPreviewAccumCount(mod));
                    const fillPct = Math.min(100, (accumCount / accumThreshold) * 100);
                    const trackStyle = isV
                        ? `left:50%; top:0; transform:translateX(-50%); width:${trackPx}px; height:100%;`
                        : `left:0; top:50%; transform:translateY(-50%); width:100%; height:${trackPx}px;`;
                    html += `<div style="position:relative; width:100%; height:100%;">`;
                    html += tex.bar_r
                        ? `<div class="component-surface" style="${trackStyle} border-radius:999px; background-image:url(${tex.bar_r}); opacity:${getComponentResourceOpacity(mod, 'bar_r').toFixed(3)};"></div>`
                        : `<div class="track" style="${trackStyle}"></div>`;
                    if(fillPct > 0.01) {
                        const fillStyle = isV
                            ? `left:50%; bottom:0; transform:translateX(-50%); width:${trackPx}px; height:${fillPct.toFixed(2)}%;`
                            : `left:0; top:50%; transform:translateY(-50%); width:${fillPct.toFixed(2)}%; height:${trackPx}px;`;
                        html += tex.bar_l
                            ? `<div class="component-surface" style="${fillStyle} border-radius:999px; background-image:url(${tex.bar_l}); z-index:2; opacity:${getComponentResourceOpacity(mod, 'bar_l').toFixed(3)};"></div>`
                            : `<div class="bar-fill" style="${fillStyle}; z-index:2;"></div>`;
                    }
                    html += `<div style="position:absolute; right:6px; top:6px; z-index:22; font-size:10px; line-height:1; color:#ffc9d1; background:rgba(5,16,28,0.72); border:1px solid rgba(255,95,116,0.28); border-radius:999px; padding:3px 6px; pointer-events:none;">${formatPreviewVariableValue(accumCount)} / ${formatPreviewVariableValue(accumThreshold)}</div>`;
                    html += `</div>`;
               }
           }

           mDiv.innerHTML = html + `</div>`;
            mDiv.onmousedown = (e) => {
                if(workspaceMode === 'run') { e.stopPropagation(); return; }
                if(e.altKey && cycleSelectionAtPoint(e.clientX, e.clientY, e.ctrlKey)) {
                    e.stopPropagation();
                    return;
                }
                startDrag(e, mod, mDiv);
            };
            mDiv.onclick = (e) => {
                e.stopPropagation();
                if(workspaceMode === 'run') {
                    if(mod.type === 'text') {
                        applyPreviewTextClickBinding(mod);
                        handleDialogueTextEvent(mod.id, 'click');
                        renderAll();
                    } else if(mod.type === 'toggle') {
                        handlePreviewToggleClick(mod);
                    }
                    return;
                }
                const ref = makeEntityRef('component', mod.id);
                if(dragSession && dragSession.targetKey === entityRefKey(ref) && dragSession.suppressClick) {
                    dragSession = null;
                    return;
                }
                if(mod.type === 'text') applyPreviewTextClickBinding(mod);
                selectEntity(ref, e.ctrlKey);
                dragSession = null;
            };
            workArea.appendChild(mDiv);
        });
        renderHierarchyPanel();
    } catch(e) {
        console.error('renderAll error:', e);
        if(workArea) workArea.innerHTML = `<div style="color:#ff5f74;padding:20px;font-size:14px;text-align:center;">渲染错误: ${escapeHtml(String(e && e.message || e))}</div>`;
    }
    }
    function mkDiv(id, cls, obj) {
        let d = document.createElement('div'); d.id=id; d.className=cls;
        d.style.left=(obj.x*100)+'%'; d.style.top=(obj.y*100)+'%';
        d.style.width=(obj.w*100)+'%'; d.style.height=(obj.h*100)+'%'; return d;
    }

    function isEntityRef(value) {
        return !!(value && value.id && (value.type === 'component' || value.type === 'group'));
    }

    function startDrag(e, targetRefOrComponent, el) {
        if(workspaceMode === 'run') return;
        e.stopPropagation();
        markHistoryDirty();
        const targetRef = isEntityRef(targetRefOrComponent)
            ? targetRefOrComponent
            : makeEntityRef('component', targetRefOrComponent && targetRefOrComponent.id);
        if(!targetRef) return;
        const alreadySelected = selectedEntities.some(ref => isSameEntityRef(ref, targetRef));
        if(!alreadySelected && !e.ctrlKey) {
            selectEntity(targetRef, false, true);
        } else if(e.ctrlKey && !alreadySelected) {
            selectEntity(targetRef, true, true);
        }
        const startX = e.clientX, startY = e.clientY;
        const snapCfg = getGridSnapConfig();
        const workRect = workArea.getBoundingClientRect();
        const workWidth = Math.max(1, workRect.width);
        const workHeight = Math.max(1, workRect.height);
        const dragThresholdPx = 3;
        const dragTargetComponent = targetRef.type === 'component'
            ? getComponentById(targetRef.id)
            : (getDescendantComponents(targetRef)[0] || null);
        const dockAnim = dragTargetComponent ? getEffectiveGlobalAnim(dragTargetComponent) : GLOBAL_ANIM_DEFAULTS;
        const dockSnapEnabled = !!(dockAnim && dockAnim.mode === 'edge_dock');
        dragSession = {
            targetKey: entityRefKey(targetRef),
            moved: false,
            suppressClick: false
        };
        isDraggingCanvasEntity = true;
        syncPreviewClock();
        let movingIds = new Set();
        selectedEntities.forEach(ref => {
            getEntityDragComponentSet(ref).forEach(id => movingIds.add(id));
        });
        let targets = components.filter(c => movingIds.has(c.id));
        if(targets.length === 0) {
            isDraggingCanvasEntity = false;
            dragSession = null;
            syncPreviewClock();
            return;
        }
        const move = (ev) => {
            let dx = (ev.clientX - startX) / workWidth;
            let dy = (ev.clientY - startY) / workHeight;
            const movedPx = Math.abs(ev.clientX - startX) + Math.abs(ev.clientY - startY);
            if(dragSession && movedPx >= dragThresholdPx) {
                dragSession.moved = true;
                dragSession.suppressClick = true;
            }
            const nextPositions = {};
            targets.forEach(c => {
                if(!c.initial) { c.initial = {x: c.x, y: c.y}; }
                let nx = c.initial.x + dx;
                let ny = c.initial.y + dy;
                if(snapCfg.enabled) {
                    nx = snapToGridValue(nx, snapCfg.snapX);
                    ny = snapToGridValue(ny, snapCfg.snapY);
                }
                nextPositions[c.id] = { x: nx, y: ny };
            });

            let dockDx = 0;
            let dockDy = 0;
            let dockEdge = '';
            if(dockSnapEnabled && previewPointerState.alt) {
                const movingBounds = getNormalizedBoundsForComponents(targets, nextPositions);
                const snapInfo = getDockSnapAdjustment(movingBounds, dockAnim);
                dockDx = snapInfo.dx || 0;
                dockDy = snapInfo.dy || 0;
                dockEdge = snapInfo.edge || '';
            }

            targets.forEach(c => {
                let el = document.getElementById(c.id);
                const next = nextPositions[c.id] || { x: c.x, y: c.y };
                const nx = next.x + dockDx;
                const ny = next.y + dockDy;
                c.x = nx;
                c.y = ny;
                if(el) { el.style.left = (nx*100)+'%'; el.style.top = (ny*100)+'%'; }
            });
            updateGroupOverlayPositions();
            setDockGuide(dockEdge);
            if(selectedGroup()) {
                refreshGeomInputs(getGroupNormalizedBounds(selectedGroup().id));
            } else if(selectedObj()) {
                refreshGeomInputs(selectedObj());
            }
        };
        const stop = () => { 
            targets.forEach(c => delete c.initial);
            setDockGuide('');
            window.removeEventListener('mousemove', move); 
            window.removeEventListener('mouseup', stop); 
            isDraggingCanvasEntity = false;
            if(dragSession && dragSession.moved) {
                setTimeout(() => {
                    if(dragSession) dragSession.suppressClick = false;
                }, 0);
            }
            syncPreviewClock();
            if(dragSession && dragSession.moved) markHistoryDirty();
            renderAll(); 
        };
        __onWindow('mousemove', move); __onWindow('mouseup', stop);
    }

    // ========== 积蓄条（累积统计）预览辅助 ==========
    function getPreviewComponentDragValue(targetComponent) {
        if(!targetComponent) return 0;
        if(targetComponent.type === 'joystick') {
            // 拖拽期间手柄值以 joystickDefaultX/Y 为准：runtime 值在物理/签名重建时会滞后一帧，导致累计位移少计一段。
            return {
                x: clamp(Number(targetComponent.joystickDefaultX) || 0, -1, 1),
                y: clamp(Number(targetComponent.joystickDefaultY) || 0, -1, 1)
            };
        }
        // 滑条：defVals 在 mousemove 时先于累计更新，读它才能拿到实时位移；格子/显式区间模式需换算回 0..1 与导出一致。
        const stored = Number(targetComponent.defVals && targetComponent.defVals[0]) || 0;
        if(isSliderGridMode(targetComponent)) {
            const steps = Math.max(2, targetComponent.gridSteps || 3);
            return clamp(stored / Math.max(1, steps - 1), 0, 1);
        }
        if(sliderUsesExplicitRange(targetComponent)) {
            return sliderActualToNormalized(targetComponent, stored);
        }
        return clamp(stored, 0, 1);
    }

    function getAccumBarsBoundTo(targetId) {
        return components.filter(c => c && c.type === 'accum' && Array.isArray(c.accumBindings) && c.accumBindings.some(b => b && b.targetId === targetId));
    }

    function getPreviewAccumState(component) {
        if(!component || component.type !== 'accum') return null;
        let state = previewAccumState.get(component.id);
        if(!state) {
            state = { count: 0, prevs: new Map() };
            previewAccumState.set(component.id, state);
        }
        return state;
    }

    function getPreviewAccumCount(component) {
        const state = getPreviewAccumState(component);
        return state ? state.count : 0;
    }

    function initAccumBindingPrevs(targetComponent) {
        if(!targetComponent) return;
        const isJoy = targetComponent.type === 'joystick';
        const value = getPreviewComponentDragValue(targetComponent);
        getAccumBarsBoundTo(targetComponent.id).forEach(bar => {
            const state = getPreviewAccumState(bar);
            if(!state) return;
            state.prevs.set(targetComponent.id, isJoy ? { x: value.x, y: value.y } : value);
        });
    }

    function accumulateDragDistance(targetComponent) {
        if(!targetComponent) return;
        const isJoy = targetComponent.type === 'joystick';
        const value = getPreviewComponentDragValue(targetComponent);
        getAccumBarsBoundTo(targetComponent.id).forEach(bar => {
            const state = getPreviewAccumState(bar);
            if(!state) return;
            const prev = state.prevs.get(targetComponent.id);
            if(prev === undefined) {
                state.prevs.set(targetComponent.id, isJoy ? { x: value.x, y: value.y } : value);
                return;
            }
            const delta = isJoy
                ? Math.abs(value.x - prev.x) + Math.abs(value.y - prev.y)
                : Math.abs(value - prev);
            state.count += delta;
            state.prevs.set(targetComponent.id, isJoy ? { x: value.x, y: value.y } : value);
            checkPreviewAccumThreshold(bar, state);
        });
    }

    function checkPreviewAccumThreshold(bar, state) {
        const threshold = Math.max(0.0001, Number(bar.accumThreshold) || 5);
        if(!state || state.count < threshold) return;
        (Array.isArray(bar.accumTriggers) ? bar.accumTriggers : []).forEach(trigger => {
            if(trigger && trigger.var && String(trigger.var).trim()) {
                previewActionVarValues.set(trigger.var, Number.isFinite(Number(trigger.value)) ? Number(trigger.value) : 0);
            }
        });
        state.count = 0;
        state.prevs = new Map();
    }

    // 预览运行模式下点击开关：与导出 INI 行为一致（多档循环/单档翻转/切换分组互斥），并为绑定它的积蓄条计数 +1。
    function handlePreviewToggleClick(toggle) {
        if(!toggle || toggle.type !== 'toggle') return;
        if(isToggleMultiMode(toggle)) {
            const steps = Math.max(1, toggle.toggleSteps || DEFAULT_TOGGLE_STEPS);
            toggle.initialValue = (Math.round(Number(toggle.initialValue) || 0) + 1) % (steps + 1);
        } else {
            toggle.initialValue = Number(toggle.initialValue) > 0 ? 0 : 1;
        }
        normalizeToggleState(toggle);
        if(toggle.switchGroup && toggle.switchGroup > 0 && Number(toggle.initialValue) > 0) {
            components.forEach(sibling => {
                if(sibling && sibling !== toggle && sibling.type === 'toggle' && sibling.switchGroup === toggle.switchGroup) {
                    sibling.initialValue = 0;
                    normalizeToggleState(sibling);
                }
            });
        }
        getAccumBarsBoundTo(toggle.id).forEach(bar => {
            const state = getPreviewAccumState(bar);
            if(state) {
                state.count += 1;
                checkPreviewAccumThreshold(bar, state);
            }
        });
        renderAll();
    }

    function startSliderAdjust(e, component) {
        e.preventDefault();
        
        // 标记正在拖拽（用于联动组件：拖拽时跳过联动覆盖）
        component.__linkedDragging = true;
        initAccumBindingPrevs(component);
        
        // 获取组件DOM元素的实际边界（局部坐标）
        const compEl = document.getElementById(component.id);
        if(!compEl) return;
        const compBounds = compEl.getBoundingClientRect();
        const isV = component.type === 'slider_v';
        // 计算轨道行程范围（减去手柄宽/高），与 INI 导出公式一致
        const handleEl = compEl.querySelector('[id^="handle_"]');
        const hSize = handleEl ? (isV ? handleEl.offsetHeight : handleEl.offsetWidth) : 0;
        const trackRange = Math.max(1, (isV ? compBounds.height : compBounds.width) - hSize);
        const hHalf = hSize * 0.5;
        // 从鼠标相对于组件DOM元素的位置计算滑块值
        const calcValue = (clientX, clientY) => {
            const relX = clientX - compBounds.left;
            const relY = clientY - compBounds.top;
            const v = isV
                ? 1 - clamp((relY - hHalf) / trackRange, 0, 1)
                : clamp((relX - hHalf) / trackRange, 0, 1);
            if(component.paramMode === '2') return clamp(v, 0, 1);
            if(isSliderGridMode(component)) {
                const gridSteps = Math.max(2, component.gridSteps || 3);
                const gridIdx = clamp(Math.round(v * (gridSteps - 1)), 0, gridSteps - 1);
                return gridIdx / (gridSteps - 1);
            }
            return clamp(v, 0, 1);
        };
        const newVal = calcValue(e.clientX, e.clientY);
        // 更新默认值（只改滑块值不改位置）
        if(!component.defVals) component.defVals = [];
        component.defVals[0] = component.paramMode === '3'
            ? Math.round(newVal * Math.max(1, (component.gridSteps || 3) - 1))
            : (sliderUsesExplicitRange(component) ? sliderNormalizedToActual(component, newVal) : newVal);
        // 如有物理模拟，同步更新运行时值
        const runtime = previewInteractiveRuntime.get(component.id);
        if(runtime && typeof runtime.value !== 'undefined') runtime.value = newVal;
        markHistoryDirty();
        renderAll();
        // 追踪鼠标移动以连续调整
        const moveHandler = (ev) => {
            const nextVal = calcValue(ev.clientX, ev.clientY);
            component.defVals[0] = component.paramMode === '3'
                ? Math.round(nextVal * Math.max(1, (component.gridSteps || 3) - 1))
                : (sliderUsesExplicitRange(component) ? sliderNormalizedToActual(component, nextVal) : nextVal);
            if(runtime && typeof runtime.value !== 'undefined') runtime.value = nextVal;
            accumulateDragDistance(component);
            renderAll();
        };
        const upHandler = () => {
            component.__linkedDragging = false;
            window.removeEventListener('mousemove', moveHandler);
            window.removeEventListener('mouseup', upHandler);
        };
        __onWindow('mousemove', moveHandler);
        __onWindow('mouseup', upHandler);
    }

    function startJoystickAdjust(e, component) {
        e.preventDefault();
        component.__linkedDragging = true;
        initAccumBindingPrevs(component);
        const compEl = document.getElementById(component.id);
        if(!compEl) return;
        const compBounds = compEl.getBoundingClientRect();
        const hMetrics = getComponentHandleMetrics(component);
        const workSize = getWorkAreaPixelSize();
        const handleW = hMetrics.width / Math.max(workSize.width, 1);
        const handleH = hMetrics.height / Math.max(workSize.height, 1);
        const rect = getComponentPixelRect(component);
        const travelX = Math.max(0, rect.width - hMetrics.width);
        const travelY = Math.max(0, rect.height - hMetrics.height);
        const hHalfX = handleW * 0.5;
        const hHalfY = handleH * 0.5;

        const calcVector = (clientX, clientY) => {
            const relX = (clientX - compBounds.left) / compBounds.width;
            const relY = (clientY - compBounds.top) / compBounds.height;
            // 映射到 -1..1
            let vx = (relX - 0.5) * 2;
            let vy = (0.5 - relY) * 2;  // Y 方向翻转
            vx = clamp(vx, -1, 1);
            vy = clamp(vy, -1, 1);
            return clampJoystickVectorToRoundedBounds(component, vx, vy);
        };

        let vec = calcVector(e.clientX, e.clientY);
        component.joystickDefaultX = vec.x;
        component.joystickDefaultY = vec.y;
        const runtime = previewInteractiveRuntime.get(component.id);
        if(runtime) { runtime.valueX = vec.x; runtime.valueY = vec.y; }
        markHistoryDirty();
        renderAll();

        const moveHandler = (ev) => {
            let v = calcVector(ev.clientX, ev.clientY);
            component.joystickDefaultX = v.x;
            component.joystickDefaultY = v.y;
            if(runtime) { runtime.valueX = v.x; runtime.valueY = v.y; }
            accumulateDragDistance(component);
            renderAll();
        };
        const upHandler = () => {
            component.__linkedDragging = false;
            window.removeEventListener('mousemove', moveHandler);
            window.removeEventListener('mouseup', upHandler);
        };
        __onWindow('mousemove', moveHandler);
        __onWindow('mouseup', upHandler);
    }

    // 滑块/摇杆手柄拦截全局函数：从 HTML 内联 onmousedown 调用
    window._svCapture = (e, compId) => {
        e.stopImmediatePropagation();
        e.preventDefault();
        const c = components.find(x => x.id === compId);
        if(!c) return;
        if(c.type === 'slider_h' || c.type === 'slider_v') startSliderAdjust(e, c);
        else if(c.type === 'joystick') startJoystickAdjust(e, c);
    };

    function downloadTextFile(content, filename) {
        const blob = new Blob([content], { type: 'text/plain;charset=utf-8' });
        const url = URL.createObjectURL(blob);
        const a = document.createElement('a');
        a.href = url;
        a.download = filename;
        document.body.appendChild(a);
        a.click();
        a.remove();
        setTimeout(() => URL.revokeObjectURL(url), 0);
    }


        // ---- SSMT4 host bridge (Vue 直连,替代 postMessage) ----
    const ssmtHostBridge = createDirectSsmHostBridge(root);
function buildDialogueIniRuntime() {
        const errors = validateDialogueLogic();
        if(errors.length) throw new Error(`文本蓝图无效：\n${errors.join('\n')}`);
        const controlledIds = new Set();
        (dialogueLogic.main.nodes || []).filter(node => node.type === 'text').forEach(node => { if(node.config && node.config.componentId) controlledIds.add(node.config.componentId); });
        dialogueLogic.dialogues.forEach(dialogue => (dialogue.nodes || []).filter(node => node.type === 'step').forEach(node => (node.config && node.config.textIds || []).forEach(id => controlledIds.add(id))));
        [dialogueLogic.main, ...dialogueLogic.dialogues].forEach(graph => (graph.nodes || []).filter(node => node.type === 'random').forEach(node => {
            if(node.config && node.config.sourceTextId) controlledIds.add(node.config.sourceTextId);
            (node.config && node.config.branches || []).forEach(branch => { if(branch.targetTextId) controlledIds.add(branch.targetTextId); });
        }));
        const componentIndex = new Map(components.map((component, index) => [component.id, index]));
        // 生命周期（外部驱动文本）：未接入文本蓝图、显示变量由外部触发器（动画驱动等）直接改写的
        // 文本框，也同样需要在打开后按 lifetimeSeconds 自动关闭。这类文本不在 controlledIds 内，
        // 走独立的边沿检测 + 倒计时（用内置 time 计时，不受 $help 门控，保证默认固定的常驻面板
        // 在 Home 隐藏帮助层时也能正常到期关闭）。
        const externalLifetimeIds = components.filter(component =>
            component && component.type === 'text' && isTextVisibilityEnabled(component) &&
            !controlledIds.has(component.id) && (Math.max(0, Number(component.lifetimeSeconds) || 0) > 0)
        ).map(component => component.id);
        const declarations = ['$dlg_clock = 0', '$dlg_prev_time = 0', '$dlg_dt = 0'];
        const reset = ['$dlg_clock = 0', '$dlg_prev_time = $time'];
        const userVariables = new Map(dialogueLogic.variables.map(item => [item.name, Number(item.initialValue) || 0]));
        const clickCodeByComponentId = new Map();
        const suffix = value => (hashStableString(String(value)) >>> 0).toString(36);
        const nodeVar = (prefix, node) => `$dlg_${prefix}_${suffix(node.id)}`;
        const dialogVar = (prefix, dialogue) => `$dlg_${prefix}_${suffix(dialogue.id)}`;
        const textVars = componentId => {
            const index = componentIndex.get(componentId);
            return { seen: `$dlg_text_seen_${index}`, last: `$dlg_text_last_${index}`, expires: `$dlg_text_exp_${index}` };
        };
        const indentLines = (lines, count) => lines.map(line => `${' '.repeat(count)}${line}`);
        const conditionExpr = cfg => {
            const clauses = (cfg && cfg.clauses || []).map(clause => `${sanitizeIniVarToken(clause.variable, '$invalid')} ${['==','!=','>','>=','<','<='].includes(clause.operator) ? clause.operator : '=='} ${Number(clause.value) || 0}`);
            return clauses.length ? clauses.map(item => `(${item})`).join(cfg && cfg.mode === 'any' ? ' || ' : ' && ') : '0 == 1';
        };
        const outgoing = (graph, nodeId, port = null) => (graph.edges || []).filter(edge => edge.fromNodeId === nodeId && (port == null || edge.fromPort === port));
        const findMembership = componentId => {
            for(const dialogue of dialogueLogic.dialogues) for(const step of (dialogue.nodes || []).filter(node => node.type === 'step')) if((step.config && step.config.textIds || []).includes(componentId)) return { dialogue, step };
            return null;
        };
        const closeStep = (dialogue, step) => {
            const lines = [];
            (step.config && step.config.textIds || []).forEach(id => { const component = getComponentById(id); if(component) lines.push(`${getTextVisibilityVar(component)} = 0`); });
            lines.push(`${dialogVar('step', dialogue)} = 0`);
            return lines;
        };
        const triggerText = componentId => {
            const component = getComponentById(componentId); if(!component) return [];
            const vars = textVars(componentId), cooldown = Math.max(0, Number(component.triggerCooldownSeconds) || 0), lifetime = Math.max(0, Number(component.lifetimeSeconds) || 0);
            const lines = [];
            if(cooldown > 0) lines.push(`if ${vars.seen} == 0 || $dlg_clock - ${vars.last} >= ${cooldown.toFixed(6)}`);
            lines.push(`${cooldown > 0 ? '    ' : ''}${getTextVisibilityVar(component)} = 1`);
            lines.push(`${cooldown > 0 ? '    ' : ''}${vars.seen} = 1`);
            lines.push(`${cooldown > 0 ? '    ' : ''}${vars.last} = $dlg_clock`);
            lines.push(`${cooldown > 0 ? '    ' : ''}${vars.expires} = ${lifetime > 0 ? `$dlg_clock + ${lifetime.toFixed(6)}` : '0'}`);
            if(cooldown > 0) lines.push('endif');
            return lines;
        };
        const randomNodes = [];
        [dialogueLogic.main, ...dialogueLogic.dialogues].forEach(graph => (graph.nodes || []).filter(node => node.type === 'random').forEach(node => randomNodes.push(node)));
        randomNodes.forEach(node => { const variable = nodeVar('rand', node); declarations.push(`${variable} = ${(Number(node.config && node.config.seed) || .3187).toFixed(6)}`); reset.push(`${variable} = ${(Number(node.config && node.config.seed) || .3187).toFixed(6)}`); });
        controlledIds.forEach(id => {
            const component = getComponentById(id); if(!component) return; const vars = textVars(id);
            declarations.push(`${vars.seen} = 0`, `${vars.last} = 0`, `${vars.expires} = 0`);
            reset.push(`${vars.seen} = 0`, `${vars.last} = 0`, `${vars.expires} = 0`, `${getTextVisibilityVar(component)} = ${component.visDefault !== false ? 1 : 0}`);
        });
        externalLifetimeIds.forEach(id => {
            const vars = textVars(id);
            declarations.push(`${vars.seen} = 0`, `${vars.expires} = 0`);
            reset.push(`${vars.seen} = 0`, `${vars.expires} = 0`);
        });
        dialogueLogic.dialogues.forEach(dialogue => {
            declarations.push(`${dialogVar('active', dialogue)} = 0`, `${dialogVar('step', dialogue)} = 0`);
            reset.push(`${dialogVar('active', dialogue)} = 0`, `${dialogVar('step', dialogue)} = 0`);
        });
        (dialogueLogic.main.nodes || []).filter(node => node.type === 'trigger').forEach(node => { declarations.push(`${nodeVar('prev', node)} = 0`); reset.push(`${nodeVar('prev', node)} = 0`); });

        const compileNode = (graph, nodeId, depth = 0) => {
            if(depth > 64) return ['; blocked instantaneous graph cycle'];
            const node = (graph.nodes || []).find(item => item.id === nodeId); if(!node) return [];
            const cfg = node.config || {}, lines = [];
            if(node.type === 'text') return triggerText(cfg.componentId);
            if(node.type === 'dialogue') {
                const dialogue = getDialogueDefinitionForGroup(cfg.groupId); if(!dialogue) return [];
                lines.push(`if ${dialogVar('active', dialogue)} == 0`, `    ${dialogVar('active', dialogue)} = 1`);
                lines.push(...indentLines(compileNode(dialogue, dialogue.entryNodeId, depth + 1), 4), 'endif'); return lines;
            }
            if(node.type === 'step') {
                if(graph.id) lines.push(`${dialogVar('step', graph)} = ${(graph.nodes || []).filter(n => n.type === 'step').findIndex(n => n.id === node.id) + 1}`);
                (cfg.textIds || []).forEach(id => lines.push(...triggerText(id))); return lines;
            }
            if(node.type === 'exit') { if(graph.id) lines.push(`${dialogVar('active', graph)} = 0`, `${dialogVar('step', graph)} = 0`); return lines; }
            if(node.type === 'action') {
                (cfg.assignments || []).forEach(action => {
                    const name = sanitizeIniVarToken(action.variable, ''); if(!name) return; const value = Number(action.value) || 0;
                    if(action.operation === 'add') lines.push(`${name} = ${name} + ${value}`);
                    else if(action.operation === 'subtract') lines.push(`${name} = ${name} - ${value}`);
                    else if(action.operation === 'toggle') lines.push(`${name} = 1 - ${name}`);
                    else lines.push(`${name} = ${value}`);
                });
            }
            if(node.type === 'condition') {
                lines.push(`if ${conditionExpr(cfg)}`);
                const yes = outgoing(graph, node.id, 'true')[0], no = outgoing(graph, node.id, 'false')[0];
                if(yes) lines.push(...indentLines(compileNode(graph, yes.toNodeId, depth + 1), 4));
                if(no) { lines.push('else'); lines.push(...indentLines(compileNode(graph, no.toNodeId, depth + 1), 4)); }
                lines.push('endif'); return lines;
            }
            if(node.type === 'random') {
                const rv = nodeVar('rand', node), edges = outgoing(graph, node.id); let cumulative = 0;
                lines.push(`${rv} = 3.91 * ${rv} * (1 - ${rv})`);
                edges.forEach((edge, index) => {
                    cumulative += Math.max(0, Number(edge.weightBp) || 0) / 10000;
                    if(index === 0) lines.push(`if ${rv} < ${cumulative.toFixed(6)}`);
                    else if(index === edges.length - 1) lines.push('else');
                    else lines.push(`else if ${rv} < ${cumulative.toFixed(6)}`);
                    lines.push(...indentLines(compileNode(graph, edge.toNodeId, depth + 1), 4));
                });
                if(edges.length) lines.push('endif'); return lines;
            }
            const edge = outgoing(graph, node.id, 'out')[0] || outgoing(graph, node.id)[0];
            if(edge) lines.push(...compileNode(graph, edge.toNodeId, depth + 1));
            return lines;
        };

        const present = ['$dlg_dt = $time - $dlg_prev_time', '$dlg_prev_time = $time', 'if $help == 1', '    if $dlg_dt >= 0 && $dlg_dt < 1', '        $dlg_clock = $dlg_clock + $dlg_dt', '    endif'];
        controlledIds.forEach(id => {
            const component = getComponentById(id), vars = textVars(id), lifetime = Math.max(0, Number(component && component.lifetimeSeconds) || 0); if(!component || lifetime <= 0) return;
            present.push(`    if ${getTextVisibilityVar(component)} == 1 && ${vars.expires} > 0 && $dlg_clock >= ${vars.expires}`, `        ${getTextVisibilityVar(component)} = 0`);
            const membership = findMembership(id); let edge = null, graph = dialogueLogic.main;
            if(membership) { graph = membership.dialogue; edge = outgoing(graph, membership.step.id, `timeout:${id}`)[0]; if(edge) present.push(...indentLines(closeStep(membership.dialogue, membership.step), 8)); }
            else { const textNode = dialogueLogic.main.nodes.find(node => node.type === 'text' && node.config && node.config.componentId === id); edge = textNode && outgoing(dialogueLogic.main, textNode.id, 'timeout')[0]; }
            if(edge) present.push(...indentLines(compileNode(graph, edge.toNodeId), 8));
            present.push('    endif');
        });
        dialogueLogic.dialogues.forEach(dialogue => {
            const steps = (dialogue.nodes || []).filter(node => node.type === 'step');
            steps.forEach((step, index) => {
                const visibility = (step.config && step.config.textIds || []).map(id => { const c = getComponentById(id); return c ? `${getTextVisibilityVar(c)} == 0` : ''; }).filter(Boolean);
                if(visibility.length) present.push(`    if ${dialogVar('active', dialogue)} == 1 && ${dialogVar('step', dialogue)} == ${index + 1} && ${visibility.join(' && ')}`, `        ${dialogVar('active', dialogue)} = 0`, `        ${dialogVar('step', dialogue)} = 0`, '    endif');
            });
        });
        (dialogueLogic.main.nodes || []).filter(node => node.type === 'trigger').forEach(node => {
            const prev = nodeVar('prev', node), edge = outgoing(dialogueLogic.main, node.id)[0];
            present.push(`    if ${conditionExpr(node.config || {})}`, `        if ${prev} == 0`);
            if(edge) present.push(...indentLines(compileNode(dialogueLogic.main, edge.toNodeId), 12));
            present.push(`        endif`, `        ${prev} = 1`, '    else', `        ${prev} = 0`, '    endif');
        });
        present.push('endif');

        // 外部驱动文本的生命周期：显示变量 0→1 边沿时盖时间戳，到点自动关闭；
        // 显示变量被外部清 0 时复位边沿标记，下次打开重新计时。
        externalLifetimeIds.forEach(id => {
            const component = getComponentById(id); if(!component) return;
            const vars = textVars(id), visVar = getTextVisibilityVar(component);
            const lifetime = Math.max(0, Number(component.lifetimeSeconds) || 0);
            present.push(
                `if ${visVar} == 1 && ${vars.seen} == 0`,
                `    ${vars.seen} = 1`,
                `    ${vars.expires} = time + ${lifetime.toFixed(6)}`,
                'endif',
                `if ${visVar} == 0`,
                `    ${vars.seen} = 0`,
                'endif',
                `if ${vars.seen} == 1 && time >= ${vars.expires}`,
                `    ${visVar} = 0`,
                'endif'
            );
        });

        controlledIds.forEach(id => {
            const lines = []; const membership = findMembership(id); const directRandom = getTextRandomNode(id);
            if(membership) lines.push(...closeStep(membership.dialogue, membership.step));
            if(directRandom && (directRandom.node.config.branches || []).length) {
                const node = directRandom.node, rv = nodeVar('rand', node), branches = node.config.branches; let cumulative = 0;
                lines.push(`${rv} = 3.91 * ${rv} * (1 - ${rv})`);
                branches.forEach((branch, index) => {
                    cumulative += Math.max(0, Number(branch.weightBp) || 0) / 10000;
                    lines.push(index === 0 ? `if ${rv} < ${cumulative.toFixed(6)}` : (index === branches.length - 1 ? 'else' : `else if ${rv} < ${cumulative.toFixed(6)}`));
                    const target = getComponentById(branch.targetTextId);
                    if(target) {
                        lines.push(`    if ${getTextVisibilityVar(target)} == 1`);
                        lines.push(`        ${getTextVisibilityVar(target)} = 0`);
                        lines.push('    else');
                        lines.push(...indentLines(triggerText(branch.targetTextId), 8));
                        lines.push('    endif');
                    }
                });
                lines.push('endif');
            } else if(membership) {
                const edge = outgoing(membership.dialogue, membership.step.id, `click:${id}`)[0]; if(edge) lines.push(...compileNode(membership.dialogue, edge.toNodeId));
            } else {
                const textNode = dialogueLogic.main.nodes.find(node => node.type === 'text' && node.config && node.config.componentId === id); const edge = textNode && outgoing(dialogueLogic.main, textNode.id, 'click')[0]; if(edge) lines.push(...compileNode(dialogueLogic.main, edge.toNodeId));
            }
            if(lines.length) clickCodeByComponentId.set(id, lines);
        });
        return { controlledIds, declarations, reset, present, userVariables, clickCodeByComponentId };
    }

    function buildGeneratedINI() {
        try {
            let resMap={}; let getRes=(rawPath)=>{let p=normalizeAssetPath(rawPath); if(p&&!resMap[p]) resMap[p]=`Resource_${p.replace(/[\/\.\s]/g,'_')}`; return resMap[p]};
            let fontResMap = {}; let fontResourceDefs = new Map(); const groupList = groups.slice(); const parentLookup = buildParentLookup();
            components.forEach(m => {
                normalizeComponentState(m, { migrateLegacyJoystick: true });
                if(m.type === 'joystick' && m.paramMode === '4') ensureJoystickDirectionState(m);
            });
            const dialogueIni = buildDialogueIniRuntime();
            const buildClickActionCode = (m, i, indent) => {
                let out = '';
                if(m.type === 'toggle') {
                    if(isToggleMultiMode(m)) {
                        out += `${indent}$val_${i} = $val_${i} + 1\n`;
                        out += `${indent}if $val_${i} > $max_${i}\n${indent}    $val_${i} = 0\n${indent}endif\n`;
                    } else {
                        out += `${indent}$val_${i} = 1 - $val_${i}\n`;
                    }
                    const isSingleToggleMode = !isToggleMultiMode(m);
                    let myVars = m.vars.flatMap(s => splitVarStr(s));
                    if(myVars.length === 0) myVars = [`$val_${i}`];
                    // 点击推送与 Sync Bindings 同级受 bindingExpr（组绑定模式）门控：bind mode 关闭时
                    // 点击只翻转面板自身 $val，不再写绑定变量（否则 $auto 会被无条件改写，A6 修复）
                    const clickBindingExpr = (componentExportMeta && componentExportMeta[i] && componentExportMeta[i].bindingExpr) || '1';
                    const emitClickVarPushes = (targetVars, extraIndent) => {
                        targetVars.forEach(v => {
                            if(v === `$val_${i}`) return;
                            out += isSingleToggleMode && m.toggleInvert === true ? `${indent}${extraIndent}${v} = 1 - $val_${i}\n` : `${indent}${extraIndent}${v} = $val_${i}\n`;
                        });
                    };
                    if(clickBindingExpr === '1') {
                        emitClickVarPushes(myVars, '');
                    } else {
                        out += `${indent}if ${clickBindingExpr}\n`;
                        emitClickVarPushes(myVars, '    ');
                        out += `${indent}endif\n`;
                    }
                   if(m.switchGroup && m.switchGroup > 0) {
                       out += `${indent}if $val_${i} > 0\n`;
                       components.forEach((sibling, k) => {
                           if(k !== i && sibling.type === 'toggle' && sibling.switchGroup === m.switchGroup) {
                               out += `${indent}    $val_${k} = 0\n`;
                               let vars = sibling.vars.flatMap(s => splitVarStr(s));
                               if(vars.length === 0) vars = [`$val_${k}`];
                               const siblingVarLines = [];
                               vars.forEach(v => {
                                   if(v === `$val_${k}`) return;
                                   const sibVal = sibling.toggleInvert === true ? 1 : 0;
                                   siblingVarLines.push(`${indent}    ${v} = ${sibVal}\n`);
                               });
                               if(siblingVarLines.length > 0) {
                                   // 兄弟开关的外部变量写入同样受 bindingExpr 门控（与 Sync Bindings 一致，A6 修复）
                                   if(clickBindingExpr === '1') {
                                       out += siblingVarLines.join('');
                                   } else {
                                       out += `${indent}    if ${clickBindingExpr}\n`;
                                       siblingVarLines.forEach(line => { out += line; });
                                       out += `${indent}    endif\n`;
                                   }
                               }
                           }
                       });
                       out += `${indent}endif\n`;
                   }
                    // 积蓄条：开关每次点击计 1
                    components.forEach((accComp, accIdx) => {
                        if(!accComp || accComp.type !== 'accum') return;
                        (Array.isArray(accComp.accumBindings) ? accComp.accumBindings : []).forEach(b => {
                            if(b && b.targetId === m.id) {
                                out += `${indent}$acc_count_${accIdx} = $acc_count_${accIdx} + 1\n`;
                            }
                        });
                    });
               } else if(m.type === 'text') {
                    const clickVar = getTextClickVar(m);
                    if(clickVar) out += `${indent}${clickVar} = 1 - ${clickVar}\n`;
                    const dialogueClickLines = dialogueIni.clickCodeByComponentId.get(m.id) || [];
                    dialogueClickLines.forEach(line => { out += `${indent}${line}\n`; });
                }
               return out;
           };
            // 积蓄条：在拖拽认领时记录手柄当前值，作为位移增量基准（仅滑条/摇杆）
            const buildAccumClaimInitCode = (srcComp, srcIdx, indent) => {
                let out = '';
                if(!srcComp || (srcComp.type !== 'slider_h' && srcComp.type !== 'slider_v' && srcComp.type !== 'joystick')) return out;
                components.forEach((accComp, accIdx) => {
                    if(!accComp || accComp.type !== 'accum') return;
                    (Array.isArray(accComp.accumBindings) ? accComp.accumBindings : []).forEach((b, bi) => {
                        if(!b || b.targetId !== srcComp.id) return;
                        if(srcComp.type === 'joystick') {
                            out += `${indent}$acc_prevx_${accIdx}_${bi} = $val_${srcIdx}_x\n`;
                            out += `${indent}$acc_prevy_${accIdx}_${bi} = $val_${srcIdx}_y\n`;
                        } else {
                            out += `${indent}$acc_prev_${accIdx}_${bi} = $val_${srcIdx}\n`;
                        }
                    });
                });
                return out;
            };
            // 积蓄条：拖拽期间按每帧手柄值变化量累加位移（仅鼠标拖拽，自动动画/物理回弹不计入）
            const buildAccumDragDeltaCode = (srcComp, srcIdx, indent) => {
                let out = '';
                if(!srcComp || (srcComp.type !== 'slider_h' && srcComp.type !== 'slider_v' && srcComp.type !== 'joystick')) return out;
                components.forEach((accComp, accIdx) => {
                    if(!accComp || accComp.type !== 'accum') return;
                    (Array.isArray(accComp.accumBindings) ? accComp.accumBindings : []).forEach((b, bi) => {
                        if(!b || b.targetId !== srcComp.id) return;
                        if(srcComp.type === 'joystick') {
                            out += `${indent}$temp = $val_${srcIdx}_x - $acc_prevx_${accIdx}_${bi}\n`;
                            out += `${indent}if $temp < 0\n${indent}    $temp = -$temp\n${indent}endif\n`;
                            out += `${indent}$acc_count_${accIdx} = $acc_count_${accIdx} + $temp\n`;
                            out += `${indent}$temp = $val_${srcIdx}_y - $acc_prevy_${accIdx}_${bi}\n`;
                            out += `${indent}if $temp < 0\n${indent}    $temp = -$temp\n${indent}endif\n`;
                            out += `${indent}$acc_count_${accIdx} = $acc_count_${accIdx} + $temp\n`;
                            out += `${indent}$acc_prevx_${accIdx}_${bi} = $val_${srcIdx}_x\n`;
                            out += `${indent}$acc_prevy_${accIdx}_${bi} = $val_${srcIdx}_y\n`;
                        } else {
                            out += `${indent}$temp = $val_${srcIdx} - $acc_prev_${accIdx}_${bi}\n`;
                            out += `${indent}if $temp < 0\n${indent}    $temp = -$temp\n${indent}endif\n`;
                            out += `${indent}$acc_count_${accIdx} = $acc_count_${accIdx} + $temp\n`;
                            out += `${indent}$acc_prev_${accIdx}_${bi} = $val_${srcIdx}\n`;
                        }
                    });
                });
                return out;
            };
           const componentExportMeta = buildExportComponentMeta(parentLookup);
            const groupOffsetOwnerById = new Map(groupList.map(group => {
                const ancestorIds = getAncestorGroupIdsForEntity(makeEntityRef('group', group.id), parentLookup);
                return [group.id, ancestorIds.length > 0 ? ancestorIds[0] : group.id];
            }));
            const textRuntimeVars = new Map();
            components.forEach(component => {
                if(!isTextVisibilityEnabled(component)) return;
                const variable = getTextVisibilityVar(component);
                if(!textRuntimeVars.has(variable)) textRuntimeVars.set(variable, component.visDefault !== false ? 1 : 0);
            });
            components.forEach(component => {
                const variable = getTextClickVar(component);
                if(variable && !textRuntimeVars.has(variable)) textRuntimeVars.set(variable, 0);
            });
            const animatedGroups = new Map();
            groupList.forEach(group => {
                if(!group || !group.globalAnim) return;
                const profile = getGlobalAnimExportProfile(group);
                if(profile.modeCode > 0) animatedGroups.set(group.id, profile);
            });
            
            // 1. GATHER ALL VARS FOR DECLARATION
            let declaredVars = new Set();
            let linkageTempVars = new Set();
            const addDeclaredVar = (value) => {
                const name = String(value || '').trim();
                if(name && name.startsWith('$')) declaredVars.add(name);
            };
            const addLinkageTempVar = (name) => {
                if(name && name.startsWith('$')) linkageTempVars.add(name);
            };
            ['$rtemp_x', '$rtemp_y', '$tempL', '$tempR'].forEach(addLinkageTempVar);
            if(components.some(isToggleMultiMode)) {
                getRes(TOGGLE_PROGRESS_ASSETS.off);
                getRes(TOGGLE_PROGRESS_ASSETS.on);
            }
            components.forEach(m=>{ 
                if(m.paths && m.paths.bg) getRes(m.paths.bg);
                // 格子档位刻度白图需在资源块输出前注册（刻度绘制代码在渲染队列阶段才执行）
                if(isSliderGridMode(m)) getRes(DEFAULT_ASSET_PATHS.gridTick);
                if(Array.isArray(m.vars)) {
                    if(isSliderGridMode(m)) getSliderGridMainVars(m).forEach(addDeclaredVar);
                    else m.vars.forEach(v => splitVarStr(v).forEach(addDeclaredVar));
                }
                if(m.type === 'text') {
                    addDeclaredVar(m.valVar);
                     
                    // Resources
                    let overrides = m.colorOverrides || {};
                    let defCol = m.fontColor || '#ffffff';
                    getComponentRenderedTextTokens(m, '000').forEach(token => {
                        if(token.char === '\n' || !token.char.trim()) return;
                        const col = token.isDynamic ? defCol : (overrides[token.sourceIndex] || defCol);
                        const resKey = getTextGlyphResourceKey(m, token.char, col);
                        fontResourceDefs.set(resKey, `${resKey}.png`);
                    });
                    if(m.valVar) {
                        for(let i=0; i<=9; i++) {
                            const s = i.toString();
                            const resKey = getTextGlyphResourceKey(m, s, defCol);
                            fontResourceDefs.set(resKey, `${resKey}.png`);
                        }
                    }
                } else if (m.type === 'sequence') {
                    addDeclaredVar(m.seqVar);
                    m.frames.forEach(f => { if(f.path) getRes(f.path); });
                } else { 
                    if(m.type === 'toggle' && isToggleMultiMode(m)) {
                        if(m.paths && m.paths.prog_off) getRes(m.paths.prog_off);
                        if(m.paths && m.paths.prog_on) getRes(m.paths.prog_on);
                    } else {
                        for(let k in m.paths) if(k!=='bg') getRes(m.paths[k]);
                        if(m.type === 'joystick' && m.paths && m.paths.post_marker) getRes(m.paths.post_marker);
                        if(m.type === 'joystick' && Array.isArray(m.linkedSlaves) && m.linkedSlaves.some(link => link && link.enabled && link.postEnabled === true)) {
                            getRes((m.paths && (m.paths.post || m.paths.post_marker)) ? (m.paths.post || m.paths.post_marker) : DEFAULT_ASSET_PATHS.postMarker);
                        }
                    }
                }
                
                // Collect depTarget vars
                if(m.depTargets) {
                    m.depTargets.forEach(targets => {
                        if(targets && targets.length > 0) {
                            targets.forEach(t => {
                                addDeclaredVar(t.var);
                            });
                        }
                    });
                }
                if(m.type === 'accum') {
                    (Array.isArray(m.accumTriggers) ? m.accumTriggers : []).forEach(tr => {
                        if(tr && tr.var) addDeclaredVar(tr.var);
                    });
                }
                if(m.gridDepTargets) {
                    m.gridDepTargets.forEach(targets => {
                        if(targets && targets.length > 0) {
                            targets.forEach(t => {
                                addDeclaredVar(t.var);
                            });
                        }
                    });
                }
                getComponentDependencyTriggerSpecs(m, components.indexOf(m)).forEach(spec => addDeclaredVar(spec.stateVar));
                if(Array.isArray(m.linkedSlaves)) {
                    m.linkedSlaves.forEach((link, linkIdx) => {
                        getLinkedSlaveConfiguredActions(link, 'enter').forEach(action => addDeclaredVar(action.var));
                        getLinkedSlaveConfiguredActions(link, 'leave').forEach(action => addDeclaredVar(action.var));
                        addLinkageTempVar(`$src_scalar_${components.indexOf(m)}_${linkIdx}`);
                        addLinkageTempVar(`$src_abs_x_${components.indexOf(m)}_${linkIdx}`);
                        addLinkageTempVar(`$src_abs_y_${components.indexOf(m)}_${linkIdx}`);
                        for(let qi = 0; qi < 4; qi++) addLinkageTempVar(`$quad_cross_${components.indexOf(m)}_${linkIdx}_${qi}`);
                        addLinkageTempVar(`$quad_fx_${components.indexOf(m)}_${linkIdx}`);
                        addLinkageTempVar(`$quad_fy_${components.indexOf(m)}_${linkIdx}`);
                        addLinkageTempVar(`$quad_j00_${components.indexOf(m)}_${linkIdx}`);
                        addLinkageTempVar(`$quad_j01_${components.indexOf(m)}_${linkIdx}`);
                        addLinkageTempVar(`$quad_j10_${components.indexOf(m)}_${linkIdx}`);
                        addLinkageTempVar(`$quad_j11_${components.indexOf(m)}_${linkIdx}`);
                        addLinkageTempVar(`$quad_det_${components.indexOf(m)}_${linkIdx}`);
                    });
                }
                if(Array.isArray(m.rangeTriggers)) {
                    m.rangeTriggers.forEach((trigger, triggerIdx) => {
                        normalizeLinkedSlaveActionList(trigger.enterActions).forEach(action => addDeclaredVar(action.var));
                        normalizeLinkedSlaveActionList(trigger.leaveActions).forEach(action => addDeclaredVar(action.var));
                        for(let qi = 0; qi < 4; qi++) addLinkageTempVar(`$range_quad_${components.indexOf(m)}_${triggerIdx}_${qi}`);
                    });
                }
            });
            const existingVisibilityVariables = new Set([
                ...groupList.map(group => group && group.visVar).filter(Boolean),
                ...textRuntimeVars.keys()
            ]);
            const preDialogueDeclaredVars = new Set(declaredVars);
            dialogueIni.userVariables.forEach((value, name) => { if(!existingVisibilityVariables.has(name)) addDeclaredVar(name); });

            const hash = document.getElementById('char_hash').value || 'c209c22b';
            const matchIndex = document.getElementById('match_index').value;
            const matchFirstIndex = document.getElementById('match_first_index').value;
            const userAspect = parseFloat(document.getElementById('global_aspect').value) || 1.777;
            const persistentAnim = getPersistentAnimSettings();
            const shortcuts = getShortcutSettings();
            const rad = (d) => (d * Math.PI / 180).toFixed(6);
            const iniNum = (value, digits = 6) => autoNumber(value, 0).toFixed(digits);
            const getComponentBoundVars = (m) => {
                if(!m || m.type === 'static' || m.type === 'text' || m.type === 'sequence') return [];
                if(m.type === 'toggle') return (m.vars || []).flatMap(v => splitVarStr(v));
                if(m.type.includes('slider')) {
                    if(isSliderSubdivisionMode(m)) return (m.vars || []).flatMap(v => splitVarStr(v));
                    if(m.paramMode === '2') return (m.vars || []).flatMap(v => splitVarStr(v));
                    if(isSliderGridMode(m)) return getSliderGridMainVars(m);
                    return splitVarStr((m.vars[0] && m.vars[0].trim()) ? m.vars[0].trim() : '');
                }
                if(m.type === 'joystick') {
                    return (m.vars || []).flatMap(v => splitVarStr(v));
                }
                return [];
            };
            const linkedTargetComponentIds = new Set();
            components.forEach(srcComp => {
                const slaves = Array.isArray(srcComp.linkedSlaves) ? srcComp.linkedSlaves : [];
                slaves.forEach(link => {
                    if(link && link.enabled && link.targetId) linkedTargetComponentIds.add(link.targetId);
                });
            });
            const bindingComponentOrder = components
                .map((component, index) => ({ component, index }))
                .sort((a, b) => {
                    const aLinked = linkedTargetComponentIds.has(a.component.id) ? 1 : 0;
                    const bLinked = linkedTargetComponentIds.has(b.component.id) ? 1 : 0;
                    return aLinked - bLinked || a.index - b.index;
                });
            const sharedBindingFlags = new Array(components.length).fill(false);
            const hasAnyTargetBindings = (targetGroups) => Array.isArray(targetGroups) && targetGroups.some(targets =>
                Array.isArray(targets) && targets.some(target => target && target.var && String(target.var).trim())
            );
            const componentExportUsage = components.map((m, i) => {
                const physicsEnabled = componentSupportsPhysics(m) && !!m.physics;
                const autoEnabled = physicsEnabled && !!m.autoAnimate;
                const localAnimProfile = getLocalAnimExportProfile(m);
                const componentGlobalAnimProfile = componentExportMeta[i].animGroup ? null : getGlobalAnimExportProfile(m);
                return {
                    physicsEnabled,
                    autoEnabled,
                    localAnimProfile,
                    localAnimEnabled: localAnimProfile.modeCode > 0,
                    componentGlobalAnimProfile: componentGlobalAnimProfile && componentGlobalAnimProfile.modeCode > 0 ? componentGlobalAnimProfile : null,
                    componentGlobalAnimEnabled: !!(componentGlobalAnimProfile && componentGlobalAnimProfile.modeCode > 0),
                    hasBindingVars: getComponentBoundVars(m).length > 0,
                    hasDepTargets: hasAnyTargetBindings(m.depTargets),
                    hasGridDepTargets: hasAnyTargetBindings(m.gridDepTargets)
                };
            });
            const hasLocalFlowAnimation = usage => !!(usage && [2, 7, 13].includes(usage.localAnimProfile.modeCode));
            const supportsPersistentSheen = (m, usage) => !!(usage && (
                usage.localAnimEnabled ||
                (m.paths && m.paths.bg) ||
                m.type === 'joystick' ||
                m.type === 'toggle' ||
                m.type === 'text'
            ));
            const hasPersistentSheen = persistentAnim.enabled && components.some((m, i) => {
                const usage = componentExportUsage[i];
                return supportsPersistentSheen(m, usage) && !hasLocalFlowAnimation(usage);
            });
            const needsFxWhite = persistentAnim.enabled && components.some((m, i) => {
                const usage = componentExportUsage[i];
                return usage && (
                    usage.localAnimEnabled ||
                    usage.componentGlobalAnimEnabled ||
                    !!componentExportMeta[i].animGroup ||
                    m.type === 'joystick' ||
                    (m.type === 'text' && m.textHoverEffect) ||
                    (persistentAnim.enabled && supportsPersistentSheen(m, usage))
                );
            });
            if(needsFxWhite) getRes(DEFAULT_ASSET_PATHS.fxWhite);
            const interactionOrder = components
                .map((m, i) => ({ i, z: Number.isFinite(Number(m.zIndex)) ? Number(m.zIndex) : 0 }))
                .sort((a, b) => (b.z - a.z) || (b.i - a.i))
                .map(item => item.i);
            const cursorXExpr = 'cursor_x';
            const cursorYExpr = 'cursor_y';
            // Components stay independent unless explicitly grouped. Reusing the same bound
            // variable name should not cause the exporter to mirror one component's runtime
            // state back into another component.
            const getBindingWriteCondition = () => null;
            const exportLayoutSignature = hashStableString({
                components: components.map(component => ({
                    id: component.id,
                    type: component.type,
                    x: Number(component.x) || 0,
                    y: Number(component.y) || 0,
                    w: Number(component.w) || 0,
                    h: Number(component.h) || 0,
                    zIndex: Number(component.zIndex) || 0,
                    rot: Number(component.rot) || 0,
                    vars: Array.isArray(component.vars) ? component.vars : [],
                    paramMode: component.paramMode || '',
                    physics: !!component.physics,
                    minVals: Array.isArray(component.minVals) ? component.minVals : [],
                    maxVals: Array.isArray(component.maxVals) ? component.maxVals : [],
                    defVals: Array.isArray(component.defVals) ? component.defVals : [],
                    initialValue: component.initialValue,
                    textVisibilityEnabled: component.textVisibilityEnabled === true,
                    visVar: component.type === 'text' ? getTextVisibilityVar(component) : '',
                    visDefault: component.visDefault !== false,
                    textHoverEffect: component.textHoverEffect === true,
                    textClickVar: component.type === 'text' ? getTextClickVar(component) : '',
                   joystickDefaultX: component.joystickDefaultX,
                   joystickDefaultY: component.joystickDefaultY,
                    accumThreshold: component.type === 'accum' ? component.accumThreshold : undefined,
                    accumDirection: component.type === 'accum' ? component.accumDirection : undefined,
                    accumBindings: component.type === 'accum' ? (component.accumBindings || []).map(b => b && b.targetId) : undefined,
                    accumTriggers: component.type === 'accum' ? (component.accumTriggers || []).map(t => t && t.var) : undefined,
               })),
                groups: groupList.map(group => ({
                    id: group.id,
                    children: Array.isArray(group.children) ? group.children : [],
                    pinned: !!group.pinned,
                    bindingEnabled: group.bindingEnabled !== false,
                    visVar: group.visVar || '',
                    visDefault: group.visDefault !== false
                })),
                roots: Array.isArray(roots) ? roots : [],
                dialogueLogic
            }) >>> 0;

            let t = `; Generated v79 (Enhanced)\n[TextureOverrideCheckHash]\nhash = ${hash}\n`;
            if (matchFirstIndex) {
                t += `match_first_index = ${matchFirstIndex}\n`;
            }
            if (matchIndex) {
                t += `match_index_count = ${matchIndex}\n`;
            }
            t += `$active = 1\n`;
            for(let p in resMap) t+=`[${resMap[p]}]\nfilename = ./res/${p}\n`;
            for (let [resKey, filename] of fontResourceDefs) t += `[Resource_Char_${resKey}]\nfilename = ./font/${filename}\n`;

            t+=`\n[KeyHelp]\nkey = ${shortcuts.help}\ntype = cycle\nrun = CommandListToggleHelp\n`;
            t+=`\n[KeyLayoutMode]\ncondition = $help == 1\nkey = ${shortcuts.layout}\ntype = cycle\nrun = CommandListToggleLayout\n`;
            
            t+=`\n[CommandListToggleHelp]\n`;
            t+=`if $help == 1\n`;
            t+=`    $help = 0\n`;
            t+=`    $is_dragging = 0\n`;
            t+=`    $drag_action = 0\n`;
            t+=`else\n`;
            t+=`    $help = 1\n`;
            t+=`endif\n`;
            
            t+=`\n[CommandListToggleLayout]\nif $layout_mode == 1\n    $layout_mode = 0\nelse\n    $layout_mode = 1\nendif\n`;
            t+=`\n[KeyResetPosition]\ncondition = $help == 1 && $active == 1\nkey = ${shortcuts.reset}\ntype = cycle\nrun = CommandListResetPosition\n`;

            t+=`[Constants]\nglobal persist $active\nglobal persist $help = 0\nglobal persist $layout_mode = 0\nglobal persist $layout_sig = 0\n`;
            t+=`global $mouse_clicked\nglobal $is_dragging\nglobal $drag_action = 0\nglobal $drag_dx\nglobal $drag_dy\nglobal $dock_modifier\nglobal $mirror_gate\nglobal persist $zoom_global = 1.0\n`;
            // 模型区域拖拽：声明面板消费的命中区域变量（由模型侧拖拽交互节点每帧 store 写入；未命中为 -1）
            [...new Set(components.filter(componentUsesZoneDrag).map(m => getZoneDragVarName(m)))].forEach(varName => {
                t+=`global ${varName} = -1\n`;
            });
            // 区域拖拽的相对抓取偏移（认领时记录光标与手柄值位置之差，拖动期间保持）
            if(components.some(componentUsesZoneDrag)) {
                t+=`global $zgrab_rx\nglobal $zgrab_ry\n`;
            }
            // 区域绑定锁存变量（每组件一个）：绑定后保持到松开，移出区域不丢；初始 0
            components.forEach((m, i) => {
                if(componentUsesZoneDrag(m)) t+=`global ${getZoneDragLatchVarName(m, i)} = 0\n`;
            });
            if(hasPersistentSheen) {
                t+=`global persist $persistent_phase = 0\nglobal $persistent_speed = ${persistentAnim.speedMultiplier.toFixed(6)}\nglobal $persistent_boost = 0.360000\n`;
            }
            t+=`global $aspect = ${userAspect}\nglobal $sw\nglobal $sh\nglobal $tt\nglobal $spring_k = 0.05\nglobal $spring_d = 0.95\nglobal $force\n`;
            t+=`global $cx\nglobal $cy\nglobal $dx\nglobal $dy\nglobal $rx\nglobal $ry\nglobal $c_rot\nglobal $s_rot\nglobal $w_draw\nglobal $h_draw\nglobal $time\n`;
            t+=`global $val_int\nglobal $d0\nglobal $d1\nglobal $d2\nglobal $d3\nglobal $d4\nglobal $d5\nglobal $d6\nglobal $d7\nglobal $d8\nglobal $d9\nglobal $d10\nglobal $d11\nglobal $d12\nglobal $d13\nglobal $d14\nglobal $d15\nglobal $temp\n\n`;
            t+=`global $prev_cursor_x\nglobal $prev_cursor_y\nglobal $cursor_delta_x\nglobal $cursor_delta_y\n\n`;

            // DECLARE USER VARIABLES
            declaredVars.forEach(v => {
                const isGraphOwned = dialogueIni.userVariables.has(v) && !preDialogueDeclaredVars.has(v);
                const visibilityGroup = groupList.find(group => group && group.visVar === v);
                const initial = visibilityGroup ? (visibilityGroup.visDefault !== false ? 1 : 0) : (isGraphOwned ? (Number(dialogueIni.userVariables.get(v)) || 0) : null);
                t+=`global persist ${v}${initial == null ? '' : ` = ${initial}`}\n`;
            });
            linkageTempVars.forEach(v => { t+=`global ${v}\n`; });
            const groupVisibilityVars = new Set(groupList.map(group => group && group.visVar).filter(Boolean));
            textRuntimeVars.forEach((defaultValue, variable) => {
                if(!declaredVars.has(variable) && !groupVisibilityVars.has(variable)) {
                    t+=`global persist ${variable} = ${defaultValue}\n`;
                }
            });
            dialogueIni.declarations.forEach(line => { t+=`global ${line}\n`; });
            t+=`\n`;

            // 建立子编组集合：有父编组的编组不生成独立拖拽偏移变量
            const childGroupIds = new Set();
            groupList.forEach(grp => {
                (grp.children || []).forEach(child => {
                    if(child.type === 'group') childGroupIds.add(child.id);
                });
            });

            if(groupList.length > 0) {
                t+=`; Group Offsets\n`;
                groupList.forEach(group => {
                    const g = group.id;
                    let gid = getGroupRuntimeVarSuffix(g);
                    const anim = animatedGroups.get(g);
                    const groupBounds = getGroupNormalizedBounds(g) || { x: 0, y: 0, w: 0, h: 0 };
                    t+=`global persist $grp_off_x_${gid} = 0\nglobal persist $grp_off_y_${gid} = 0\n`;
                    t+=`global $grp_base_x_${gid} = ${groupBounds.x.toFixed(6)}\nglobal $grp_base_y_${gid} = ${groupBounds.y.toFixed(6)}\nglobal $grp_base_w_${gid} = ${groupBounds.w.toFixed(6)}\nglobal $grp_base_h_${gid} = ${groupBounds.h.toFixed(6)}\n`;
                    if(group.visVar && !declaredVars.has(group.visVar)) t+=`global persist ${group.visVar} = ${group.visDefault !== false ? 1 : 0}\n`;
                    t+=`global persist ${getGroupPinVarName(g)} = ${group.pinned ? 1 : 0}\n`;
                    t+=`global persist ${getGroupBindingVarName(g)} = ${group.bindingEnabled !== false ? 1 : 0}\n`;
                    t+=`global persist $grp_anim_mode_${gid} = ${anim ? anim.modeCode : 0}\n`;
                    t+=`global persist $grp_anim_edge_${gid} = ${anim ? anim.edgeCode : -1}\n`;
                    t+=`global $grp_anim_str_${gid} = ${anim ? anim.strength.toFixed(6) : 0}\n`;
                    t+=`global $grp_anim_speed_${gid} = ${anim ? anim.speed.toFixed(6) : 0}\n`;
                    t+=`global $grp_anim_reveal_${gid} = ${anim ? anim.reveal.toFixed(6) : 0}\n`;
                    t+=`global $grp_anim_trigger_${gid} = ${anim ? anim.trigger.toFixed(6) : 0}\n`;
                    t+=`global $grp_anim_touch_tol_${gid} = ${anim ? anim.touchTolerance.toFixed(6) : 0}\n`;
                    t+=`global $grp_anim_ease_${gid} = ${anim ? anim.ease.toFixed(6) : 0}\n`;
                    t+=`global persist $grp_anim_prog_${gid} = 0\n`;
                    t+=`global $grp_anim_tx_${gid} = 0\nglobal $grp_anim_ty_${gid} = 0\nglobal $grp_anim_scale_${gid} = 1\nglobal $grp_anim_resolved_edge_${gid} = -1\n`;
                    t+=`global persist $grp_anim_phase_${gid} = 0\nglobal $grp_anim_wave_${gid} = 0\n`;
                });
                t+='\n';
            }

            components.forEach((m,i) => {
                const usage = componentExportUsage[i];
                if(m.type === 'joystick' && m.paramMode === '4') ensureJoystickDirectionState(m);
                const exportDefaultValue = m.type && m.type.includes('slider') && m.paramMode === '1'
                    ? getSliderStoredDefaultValue(m)
                    : ((m.defVals && m.defVals[0]) || 0);
                t+=`; --- Component ${i + 1}: ${m.type} id=${m.id} pos=(${m.x.toFixed(4)}, ${m.y.toFixed(4)}) size=(${m.w.toFixed(4)}, ${m.h.toFixed(4)}) val0=${exportDefaultValue} ---\n`;
                t+=`;   raw: x=${m.x.toFixed(6)} y=${m.y.toFixed(6)} w=${m.w.toFixed(6)} h=${m.h.toFixed(6)}\n`;
                t+=`global $base_x_${i} = ${m.x.toFixed(4)}\nglobal $base_y_${i} = ${m.y.toFixed(4)}\nglobal $base_w_${i} = ${m.w.toFixed(4)}\nglobal $base_h_${i} = ${m.h.toFixed(4)}\n`;
                t+=`global $abs_x_${i}\nglobal $abs_y_${i}\nglobal $abs_w_${i}\nglobal $abs_h_${i}\n`;
                t+=`global $rot_${i} = ${rad(m.rot||0)}\nglobal $sin_${i} = ${Math.sin((m.rot||0)*Math.PI/180).toFixed(6)}\nglobal $cos_${i} = ${Math.cos((m.rot||0)*Math.PI/180).toFixed(6)}\n`;
                t+=`global persist $off_x_${i} = 0\nglobal persist $off_y_${i} = 0\n`;
                if(usage.componentGlobalAnimEnabled) {
                    const globalAnim = usage.componentGlobalAnimProfile;
                    t+=`global persist $anim_global_mode_${i} = ${globalAnim.modeCode}\n`;
                    t+=`global persist $anim_global_edge_${i} = ${globalAnim.edgeCode}\n`;
                    t+=`global $anim_global_str_${i} = ${globalAnim.strength.toFixed(6)}\n`;
                    t+=`global $anim_global_speed_${i} = ${globalAnim.speed.toFixed(6)}\n`;
                    t+=`global $anim_global_reveal_${i} = ${globalAnim.reveal.toFixed(6)}\n`;
                    t+=`global $anim_global_trigger_${i} = ${globalAnim.trigger.toFixed(6)}\n`;
                    t+=`global $anim_global_touch_tol_${i} = ${globalAnim.touchTolerance.toFixed(6)}\n`;
                    t+=`global $anim_global_ease_${i} = ${globalAnim.ease.toFixed(6)}\n`;
                    t+=`global persist $anim_global_prog_${i} = 0\nglobal persist $anim_global_phase_${i} = 0\nglobal $anim_global_wave_${i} = 0\nglobal $anim_global_resolved_edge_${i} = -1\n`;
                }
                // $anim_global_tx/ty/scale 在坐标计算中始终使用（未编组组件回退）
                t+=`global $anim_global_tx_${i} = 0\nglobal $anim_global_ty_${i} = 0\nglobal $anim_global_scale_${i} = 1\n`;
                if(usage.localAnimEnabled) {
                    const localAnim = usage.localAnimProfile;
                    t+=`global persist $anim_local_mode_${i} = ${localAnim.modeCode}\n`;
                    t+=`global $anim_local_str_${i} = ${localAnim.strength.toFixed(6)}\n`;
                    t+=`global $anim_local_speed_${i} = ${localAnim.speed.toFixed(6)}\n`;
                    t+=`global persist $anim_local_phase_${i} = 0\n`;
                } else {
                    t+=`global $anim_local_mode_${i} = 0\nglobal $anim_local_phase_${i} = 0\n`;
                }
                // 以下变量在坐标计算和渲染中始终使用，无论动画是否启用
                t+=`global $anim_local_dx_${i} = 0\nglobal $anim_local_dy_${i} = 0\nglobal $anim_local_scale_${i} = 1\nglobal $anim_local_alpha_${i} = 1\nglobal $anim_local_rot_${i} = 0\nglobal $anim_local_sheen_${i} = 0\nglobal $anim_handle_dx_${i} = 0\nglobal $anim_handle_dy_${i} = 0\nglobal $anim_handle_scale_${i} = 1\nglobal $anim_handle_alpha_${i} = 1\nglobal $anim_fill_alpha_${i} = 1\nglobal $anim_state_dx_${i} = 0\nglobal $anim_text_wave_${i} = 0\nglobal $anim_fx_boost_${i} = 0\n`;
                if(m.type === 'text' && m.textHoverEffect) t+=`global $text_hover_${i} = 0\n`;

               if (m.type !== 'text' && m.type !== 'static' && m.type !== 'sequence') {
                    if(m.type === 'accum') {
                        const accumThreshold = Math.max(0.0001, Number(m.accumThreshold) || 5);
                        t+=`global persist $acc_count_${i} = 0\n`;
                        t+=`global $acc_fill_${i} = 0\n`;
                        t+=`global $acc_thresh_${i} = ${accumThreshold.toFixed(6)}\n`;
                        t+=`global $tt_${i} = ${safeNum(m.trackThick, DEFAULT_TT).toFixed(6)}\n`;
                        (Array.isArray(m.accumBindings) ? m.accumBindings : []).forEach((b, bi) => {
                            const tgt = components.find(c => c.id === b.targetId);
                            if(tgt && tgt.type === 'joystick') {
                                t+=`global $acc_prevx_${i}_${bi} = 0\nglobal $acc_prevy_${i}_${bi} = 0\n`;
                            } else if(tgt && (tgt.type === 'slider_h' || tgt.type === 'slider_v')) {
                                t+=`global $acc_prev_${i}_${bi} = 0\n`;
                            }
                        });
                    } else {
                   const physicsSupported = componentSupportsPhysics(m);
                    if(usage.physicsEnabled) {
                        t+=`global persist $phys_mode_${i} = 1\n`;
                        t+=`global $spring_k_${i} = ${((m.springK ?? 0.05)).toFixed(4)}\n`;
                        t+=`global $spring_d_${i} = ${((m.springD ?? 0.95)).toFixed(4)}\n`;
                        if(m.type === 'joystick') {
                            t+=`global persist $vel_${i}_x = 0\nglobal persist $vel_${i}_y = 0\n`;
                        } else {
                            t+=`global persist $vel_${i} = 0\n`;
                        }
                    }
                    
                    const autoEnabled = usage.autoEnabled;
                    const autoDefault = autoEnabled ? 1 : 0;
                    const autoSource = getAutoSourceMode(m);
                    let autoSamplesX = null;
                    let autoSamplesY = null;
                    if (usage.physicsEnabled && m.type !== 'joystick') {
                        t+=`global $gravity_${i} = ${((m.gravity ?? 0)).toFixed(4)}\n`;
                        // 始终声明 $auto（关闭时设为 0）以覆盖 persist 存档残留
                        t+=`global persist $auto_${i} = ${autoDefault}\n`;
                        // 联动占用闩锁：联动激活=1、离开=0；物理自动驱动条件组合 $auto==1 && $linked_active==0，
                        // 使联动暂停/恢复自动动画不再直接写 $auto（避免覆盖绑定开关、触发 follow 反向拉值与 persist 污染）
                        t+=`global $linked_active_${i} = 0\n`;
                        if (autoEnabled) {
                            t+=`global $auto_str_${i} = ${((m.autoStr ?? 0.1)).toFixed(4)}\n`;
                            t+=`global $auto_amp_x_${i} = ${((m.autoAmpX ?? 1)).toFixed(4)}\n`;
                            t+=`global $auto_speed_${i} = ${((m.autoSpeed ?? 0.015)).toFixed(4)}\n`;
                            t+=`global $auto_response_${i} = ${((m.autoResponse ?? 0.22)).toFixed(4)}\n`;
                            t+=`global $auto_bounce_${i} = ${((m.autoBounce ?? 0.25)).toFixed(4)}\n`;
                            t+=`global $chaos_rate_${i} = ${m.chaosRate || 96}\n`;
                            t+=`global persist $auto_prev_${i} = 0\n`;
                            if (autoSource === 'chaos') {
                                t+=`global $auto_seed_x_${i} = ${((m.autoSeedX ?? 0.3187)).toFixed(4)}\n`;
                                t+=`global persist $chaos_${i}_x = ${((m.autoSeedX ?? 0.3187)).toFixed(4)}\n`;
                                t+=`global persist $counter_${i} = ${Math.max(0, (m.chaosRate || 96) - 1)}\n`;
                            } else {
                                try {
                                    autoSamplesX = buildAutoFunctionSamples(m, 'x');
                                    if (m.type === 'joystick') autoSamplesY = buildAutoFunctionSamples(m, 'y');
                                } catch(err) {
                                    throw new Error(`组件 ${i + 1} 的自动函数无效: ${err.message}`);
                                }
                                t+=`global persist $auto_phase_${i} = 0\n`;
                            }
                        }
                    }
                    
                    if (m.type === 'joystick') {
                        let defX = clamp(typeof m.joystickDefaultX === 'number' ? m.joystickDefaultX : ((m.defVals && m.defVals[0]) || 0), -1, 1);
                        let defY = clamp(typeof m.joystickDefaultY === 'number' ? m.joystickDefaultY : ((m.defVals && m.defVals[1]) || 0), -1, 1);
                        t+=`global persist $val_${i}_x = ${defX}\nglobal persist $val_${i}_y = ${defY}\n`;
                        t+=`global $vprev_${i}_x = ${defX}\nglobal $vprev_${i}_y = ${defY}\n`;
                        t+=`global $rest_${i}_x = ${defX}\nglobal $rest_${i}_y = ${defY}\n`;
                        t+=`global $gravity_${i} = ${((m.gravity ?? 0)).toFixed(4)}\n`;
                        if(usage.physicsEnabled) {
                            t+=`global persist $auto_prev_${i} = 0\n`;
                        }
                        if (usage.physicsEnabled && m.physicsProfile === 'breast') {
                            t+=`global persist $drive_${i}_x = ${defX}\nglobal persist $drive_${i}_y = ${defY}\n`;
                        }
                        // 始终声明 $auto（关闭时设为 0）以覆盖 persist 存档残留
                        t+=`global persist $auto_${i} = ${autoDefault}\n`;
                        // 联动占用闩锁：联动激活=1、离开=0；物理自动驱动条件组合 $auto==1 && $linked_active==0，
                        // 使联动暂停/恢复自动动画不再直接写 $auto（避免覆盖绑定开关、触发 follow 反向拉值与 persist 污染）
                        t+=`global $linked_active_${i} = 0\n`;
                        if (autoEnabled) {
                            t+=`global $auto_str_${i} = ${((m.autoStr ?? 0.1)).toFixed(4)}\n`;
                            t+=`global $auto_amp_x_${i} = ${((m.autoAmpX ?? 1)).toFixed(4)}\n`;
                            t+=`global $auto_speed_${i} = ${((m.autoSpeed ?? 0.015)).toFixed(4)}\n`;
                            t+=`global $auto_response_${i} = ${((m.autoResponse ?? 0.22)).toFixed(4)}\n`;
                            t+=`global $auto_bounce_${i} = ${((m.autoBounce ?? 0.25)).toFixed(4)}\n`;
                            t+=`global $chaos_rate_${i} = ${m.chaosRate || 96}\n`;
                            t+=`global $auto_seed_x_${i} = ${((m.autoSeedX ?? 0.3187)).toFixed(4)}\n`;
                            t+=`global persist $chaos_${i}_x = ${((m.autoSeedX ?? 0.3187)).toFixed(4)}\n`;
                            t+=`global persist $counter_${i} = ${Math.max(0, (m.chaosRate || 96) - 1)}\n`;
                            t+=`global persist $auto_goal_${i}_x = ${defX}\n`;
                            t+=`global persist $auto_goal_${i}_y = ${defY}\n`;
                            t+=`global persist $auto_tgt_${i}_x = ${defX}\n`;
                            t+=`global persist $auto_tgt_${i}_y = ${defY}\n`;
                            t+=`global $auto_amp_y_${i} = ${((m.autoAmpY ?? 1)).toFixed(4)}\n`;
                            if (autoSource === 'chaos') {
                                t+=`global $auto_seed_y_${i} = ${((m.autoSeedY ?? 0.6123)).toFixed(4)}\n`;
                                t+=`global persist $chaos_${i}_y = ${((m.autoSeedY ?? 0.6123)).toFixed(4)}\n`;
                            } else {
                                try {
                                    autoSamplesX = buildAutoFunctionSamples(m, 'x');
                                    autoSamplesY = buildAutoFunctionSamples(m, 'y');
                                } catch(err) {
                                    throw new Error(`组件 ${i + 1} 的自动函数无效: ${err.message}`);
                                }
                                t+=`global persist $auto_phase_${i} = 0\n`;
                            }
                        }
                        if(m.paramMode === '4') {
                            const joyCfg = getJoystickConfig(m);
                            t+=`global $joy_w0_${i} = 0\n`;
                            t+=`global $joy_w1_${i} = 0\n`;
                            t+=`global $joy_mix_best_${i} = -1\n`;
                            t+=`global $joy_mix_proj_${i} = -1000\n`;
                            for(let dirIdx = 0; dirIdx < joyCfg.directionCount; dirIdx++) {
                                for(let segIdx = 0; segIdx < joyCfg.subdivisions; segIdx++) {
                                    const flatIdx = getJoystickDirectionVarIndex(m, dirIdx, segIdx);
                                    t+=`global $joy_dir_${i}_${flatIdx} = 0\n`;
                                    t+=`global $dprev_${i}_${flatIdx} = 0\n`;
                                }
                            }
                        } else {
                            let maxX = (m.maxVals && m.maxVals[0]) || 1;
                            let maxY = (m.maxVals && m.maxVals[1]) || 1;
                            t+=`global $max_${i}_x = ${maxX}\nglobal $max_${i}_y = ${maxY}\n`;
                        }
                    } else {
                        let def = 0;
                        let rest = 0;
                        if(m.type.includes('slider')) {
                            if(m.paramMode === '2') {
                                def = (m.defVals && m.defVals[0]) || 0;
                                if(m.defVals === undefined || m.defVals[0] === undefined) def = 0.5;
                                rest = 0.5;
                            } else if(m.paramMode === '3') {
                                def = ((m.defVals && m.defVals[0]) || 0) / Math.max(1, (m.gridSteps || 3) - 1);
                                rest = def;
                            } else {
                                def = getSliderPreviewBaseValue(m);
                                rest = def;
                            }
                        } else if(m.type === 'toggle') {
                            def = m.initialValue || 0;
                            rest = def;
                        } else {
                            def = (m.defVals && m.defVals[0]) || 0;
                            rest = def;
                        }
                        
                        t+=`global persist $val_${i} = ${def}\n`;
                        t+=`global $vprev_${i} = ${def}\n`;
                        t+=`global $rest_${i} = ${rest}\n`;
                        if(usage.physicsEnabled) {
                            t+=`global persist $vel_${i} = 0\n`;
                        }
                        if (autoEnabled) {
                            t+=`global persist $auto_goal_${i} = ${def}\n`;
                            t+=`global persist $auto_tgt_${i} = ${def}\n`;
                        }
                        let maxVal = (m.maxVals && m.maxVals[0]) || 1;
                        if(m.type === 'toggle' && isToggleMultiMode(m)) maxVal = m.toggleSteps || DEFAULT_TOGGLE_STEPS;
                        t+=`global $max_${i} = ${maxVal}\n`;
                        if(isSliderSubdivisionMode(m)) {
                            const sliderCfg = getSliderSubdivisionConfig(m);
                            for(let flatIdx = 0; flatIdx < sliderCfg.totalVars; flatIdx++) {
                                t+=`global $slider_seg_${i}_${flatIdx} = 0\n`;
                                t+=`global $segprev_${i}_${flatIdx} = 0\n`;
                            }
                        }
                    }
                    const handleMetrics = getComponentHandleMetrics(m);
                    const workSize = getWorkAreaPixelSize();
                    const handleW = handleMetrics.width / Math.max(workSize.width, 1);
                    const handleH = handleMetrics.height / Math.max(workSize.height, 1);
                    const joyCornerPx = getComponentCornerRadiusPx(m);
                    const joyHandleRadiusPx = Math.min(handleMetrics.width, handleMetrics.height) * 0.5;
                    const joyCenterCornerPx = Math.max(0, joyCornerPx - joyHandleRadiusPx);
                   t+=`global $hs_${i} = ${handleW.toFixed(6)}\nglobal $hh_${i} = ${handleH.toFixed(6)}\nglobal $tt_${i} = ${safeNum(m.trackThick, DEFAULT_TT).toFixed(6)}\nglobal $joy_corner_${i} = ${(joyCenterCornerPx / BASE_HEIGHT).toFixed(6)}\nglobal $r_hdl_${i}_x\nglobal $r_hdl_${i}_y\n`;
                    }
               }
           });

           // 碰撞桩变量声明（必须在Constants段声明，不能在Present段每帧重新声明=0）
            const linkedPostTargets = new Set();
            const linkedSlaveTriggerStates = [];
            const rangeTriggerStates = [];
            // 区间动作初始状态（默认手柄位置）用于重置后一次性应用 enter/leave
            const rangeInitEntries = [];
            components.forEach(m => {
                if(!Array.isArray(m.linkedSlaves)) return;
                m.linkedSlaves.forEach((link, linkIdx) => {
                    if(link.postEnabled && link.targetId) {
                        const tgtIdx = components.indexOf(components.find(c => c.id === link.targetId));
                        if(tgtIdx >= 0) linkedPostTargets.add(tgtIdx);
                    }
                    if(hasLinkedSlaveConfiguredActions(link)) {
                        const compIdx = components.indexOf(m);
                        linkedSlaveTriggerStates.push({ compIdx, linkIdx });
                        const enterActions = getLinkedSlaveConfiguredActions(link, 'enter');
                        const leaveActions = getLinkedSlaveConfiguredActions(link, 'leave');
                        const targetComp = link.targetId ? components.find(c => c.id === link.targetId) : null;
                        const defVal = m.type === 'joystick' ? getJoystickPreviewBaseVector(m) : getSliderPreviewBaseValue(m);
                        let defaultInside = false;
                        if(m.type === 'joystick' && getLinkedSlaveEffectiveRegionMode(link, m, targetComp) === 'rect') {
                            defaultInside = pointInConvexQuad(getLinkedSlaveRegionPoints(link), defVal.x, defVal.y);
                        } else {
                            const scalar = m.type === 'joystick' ? getPreviewLinkedScalarSourceValue(link, m, defVal) : (typeof defVal === 'number' ? defVal : 0);
                            const sMin = link.srcMin ?? 0;
                            const sMax = link.srcMax ?? 0.5;
                            defaultInside = scalar >= sMin && scalar <= sMax;
                        }
                        rangeInitEntries.push({ stateVar: `$linked_range_${compIdx}_${linkIdx}`, inside: defaultInside, enterActions, leaveActions });
                    }
                });
                // 收集独立区间触发状态变量
                if(Array.isArray(m.rangeTriggers)) {
                    m.rangeTriggers.forEach((trigger, triggerIdx) => {
                        const enterActions = Array.isArray(trigger.enterActions)
                            ? trigger.enterActions.filter(a => a && a.var && String(a.var).trim())
                            : [];
                        const leaveActions = Array.isArray(trigger.leaveActions)
                            ? trigger.leaveActions.filter(a => a && a.var && String(a.var).trim())
                            : [];
                        if(enterActions.length > 0 || leaveActions.length > 0) {
                            const compIdx = components.indexOf(m);
                            rangeTriggerStates.push({ compIdx, triggerIdx });
                            const defVal = m.type === 'joystick' ? getJoystickPreviewBaseVector(m) : getSliderPreviewBaseValue(m);
                            let defaultInside = false;
                            if(m.type === 'joystick') {
                                defaultInside = pointInConvexQuad(getRangeTriggerRegionPoints(trigger), defVal.x, defVal.y);
                            } else {
                                const scalar = typeof defVal === 'number' ? defVal : 0;
                                const sMin = trigger.srcMin ?? 0;
                                const sMax = trigger.srcMax ?? 0.5;
                                defaultInside = scalar >= sMin && scalar <= sMax;
                            }
                            rangeInitEntries.push({ stateVar: `$range_state_${compIdx}_${triggerIdx}`, inside: defaultInside, enterActions, leaveActions });
                        }
                    });
                }
            });
                linkedPostTargets.forEach(tgtIdx => {
                    // 碰撞桩锁存是运行时状态，不能用 persist（否则上次激活会残留，导致下次进入游戏未触桩也错误激活）
                    t+=`global $linked_post_${tgtIdx} = 0\n`;
                });
            linkedSlaveTriggerStates.forEach(({ compIdx, linkIdx }) => {
                t+=`global persist $linked_range_${compIdx}_${linkIdx} = 0\n`;
            });
            rangeTriggerStates.forEach(({ compIdx, triggerIdx }) => {
                t+=`global persist $range_state_${compIdx}_${triggerIdx} = 0\n`;
            });
            t+=`global $range_actions_init = 0\n`;

            t+=`\n[KeyZoomIn]\ncondition = $help == 1\nkey = ${shortcuts.zoomIn}\ntype = cycle\nrun = CommandListZoomIn\n[KeyZoomOut]\ncondition = $help == 1\nkey = ${shortcuts.zoomOut}\ntype = cycle\nrun = CommandListZoomOut\n[KeyDockModifier]\ncondition = $active == 1\nkey = ${shortcuts.dockModifier}\ntype = hold\n$dock_modifier = 1\n[KeyMouseDrag]\ncondition = $active == 1\nkey = ${shortcuts.drag}\ntype = hold\n$mouse_clicked = 1\n[KeyMouseDrag2]\ncondition = $active == 1\nkey = ${shortcuts.mouseDrag}\ntype = hold\n$mouse_clicked = 1\n[CommandListResetPosition]\n$zoom_global = 1.0\n`;
            if(groupList.length > 0) {
                groupList.forEach(group => {
                    const gid = getGroupRuntimeVarSuffix(group.id);
                    // 只重置顶层编组的偏移（子编组不声明独立的 $grp_off_x/y）
                    if(!childGroupIds.has(group.id)) {
                        t+=`$grp_off_x_${gid} = 0\n$grp_off_y_${gid} = 0\n`;
                    }
                });
            }
            components.forEach((m, i) => {
                t+=`$off_x_${i} = 0\n$off_y_${i} = 0\n`;
                // 重置滑块/摇杆值到默认，解决 persist 残留问题
                if(m.type.includes('slider')) {
                    const resetSliderVal = m.paramMode === '3'
                        ? (((m.defVals && m.defVals[0]) || 0) / Math.max(1, (m.gridSteps || 3) - 1))
                        : getSliderPreviewBaseValue(m);
                    t+=`$val_${i} = ${resetSliderVal.toFixed(4)}\n`;
                } else if(m.type === 'joystick') {
                    let dx = m.joystickDefaultX ?? ((m.defVals && m.defVals[0]) ?? 0);
                    let dy = m.joystickDefaultY ?? ((m.defVals && m.defVals[1]) ?? 0);
                    t+=`$val_${i}_x = ${clamp(dx, -1, 1).toFixed(4)}\n$val_${i}_y = ${clamp(dy, -1, 1).toFixed(4)}\n`;
               } else if(m.type === 'toggle') {
                   // 重置开关自身状态到默认值，解决 persist 残留（否则绑定的 $auto 会被旧存档拉回 1）
                   const toggleMax = isToggleMultiMode(m) ? Math.max(1, (m.toggleSteps || DEFAULT_TOGGLE_STEPS)) : 1;
                   const resetToggleVal = clamp(Math.round(Number(m.initialValue) || 0), 0, toggleMax);
                   t+=`$val_${i} = ${resetToggleVal}\n`;
                   if(!isToggleMultiMode(m)) {
                       const resetToggleOn = resetToggleVal === 1;
                       (m.vars || []).flatMap(s => splitVarStr(s)).forEach(v => {
                           if(!v || v === `$val_${i}`) return;
                           const resetVarVal = m.toggleInvert === true ? (resetToggleOn ? 0 : 1) : (resetToggleOn ? 1 : 0);
                           t+=`${v} = ${resetVarVal}\n`;
                       });
                   }
                } else if(m.type === 'accum') {
                    t+=`$acc_count_${i} = 0\n`;
               }
           });
           textRuntimeVars.forEach((defaultValue, variable) => {
               t+=`${variable} = ${defaultValue}\n`;
           });
            linkedSlaveTriggerStates.forEach(({ compIdx, linkIdx }) => {
                t+=`$linked_range_${compIdx}_${linkIdx} = 0\n`;
            });
            rangeTriggerStates.forEach(({ compIdx, triggerIdx }) => {
                t+=`$range_state_${compIdx}_${triggerIdx} = 0\n`;
            });
            t+=`$range_actions_init = 0\n`;
            t+=`[CommandListZoomIn]\n$zoom_global = $zoom_global + 0.05\n[CommandListZoomOut]\n$zoom_global = $zoom_global - 0.05\n`;

            t+=`\n[Present]\npost $active = 0\n    $time = time\n    if $layout_sig != ${exportLayoutSignature}\n`;
            t+=`        $layout_sig = ${exportLayoutSignature}\n`;
            t+=`        $help = 0\n`;
            if(groupList.length > 0) {
                groupList.forEach(group => {
                    const gid = getGroupRuntimeVarSuffix(group.id);
                    t+=`        ${getGroupPinVarName(group.id)} = ${group.pinned ? 1 : 0}\n`;
                    if(!childGroupIds.has(group.id)) {
                        t+=`        $grp_off_x_${gid} = 0\n        $grp_off_y_${gid} = 0\n`;
                    }
                });
            }
            components.forEach((m, i) => {
                t+=`        $off_x_${i} = 0\n        $off_y_${i} = 0\n`;
                if(m.type.includes('slider')) {
                    const resetSliderVal = m.paramMode === '3'
                        ? (((m.defVals && m.defVals[0]) || 0) / Math.max(1, (m.gridSteps || 3) - 1))
                        : getSliderPreviewBaseValue(m);
                    t+=`        $val_${i} = ${resetSliderVal.toFixed(4)}\n`;
                } else if(m.type === 'joystick') {
                    let dx = m.joystickDefaultX ?? ((m.defVals && m.defVals[0]) ?? 0);
                    let dy = m.joystickDefaultY ?? ((m.defVals && m.defVals[1]) ?? 0);
                    t+=`        $val_${i}_x = ${clamp(dx, -1, 1).toFixed(4)}\n        $val_${i}_y = ${clamp(dy, -1, 1).toFixed(4)}\n`;
               } else if(m.type === 'toggle') {
                   // 重置开关自身状态到默认值，解决 persist 残留（否则绑定的 $auto 会被旧存档拉回 1）
                   const toggleMax = isToggleMultiMode(m) ? Math.max(1, (m.toggleSteps || DEFAULT_TOGGLE_STEPS)) : 1;
                   const resetToggleVal = clamp(Math.round(Number(m.initialValue) || 0), 0, toggleMax);
                   t += `        $val_${i} = ${resetToggleVal}\n`;
                   if(!isToggleMultiMode(m)) {
                       const resetToggleOn = resetToggleVal === 1;
                       (m.vars || []).flatMap(s => splitVarStr(s)).forEach(v => {
                           if(!v || v === `$val_${i}`) return;
                           const resetVarVal = m.toggleInvert === true ? (resetToggleOn ? 0 : 1) : (resetToggleOn ? 1 : 0);
                           t += `        ${v} = ${resetVarVal}\n`;
                       });
                   }
                } else if(m.type === 'accum') {
                    // 积蓄条：布局签名变化时清空累计，开始新一轮统计
                    t+=`        $acc_count_${i} = 0\n`;
               }
           });
           textRuntimeVars.forEach((defaultValue, variable) => {
               t+=`        ${variable} = ${defaultValue}\n`;
           });
           dialogueIni.reset.forEach(line => { t+=`        ${line}\n`; });
            linkedPostTargets.forEach(tgtIdx => {
                t+=`        $linked_post_${tgtIdx} = 0\n`;
            });
            // 布局变化时联动占用闩锁清零（与 $linked_post 同级处理；$linked_active 为瞬时状态非 persist）
            components.forEach((m, i) => {
                if(componentExportUsage[i] && componentExportUsage[i].physicsEnabled) {
                    t+=`        $linked_active_${i} = 0\n`;
                }
            });
            linkedSlaveTriggerStates.forEach(({ compIdx, linkIdx }) => {
                t+=`        $linked_range_${compIdx}_${linkIdx} = 0\n`;
            });
            rangeTriggerStates.forEach(({ compIdx, triggerIdx }) => {
                t+=`        $range_state_${compIdx}_${triggerIdx} = 0\n`;
            });
            t+=`        $range_actions_init = 0\n`;
            t+=`    endif\n`;
            t+=`    if $active == 0\n        $help = 0\n        $is_dragging = 0\n        $drag_action = 0\n    endif\n\n    if $active == 1\n`;
            if(rangeInitEntries.length > 0) {
                // 重置/首次进入后：按默认手柄位置一次性应用各区间动作（离开优先、进入最后），
                // 保证“默认在区间内/外”的 enter/leave 初始值正确，且多区间共享变量不互相覆盖。
                t+=`    if $range_actions_init == 0\n`;
                t+=`        $range_actions_init = 1\n`;
                t+=`        ; Range Actions init (default position)\n`;
                rangeInitEntries.filter(e => !e.inside).forEach(e => {
                    t+=`        ${e.stateVar} = 0\n`;
                    if(e.leaveActions.length > 0) {
                        t += buildLinkedSlaveActionINI(e.leaveActions, '        ');
                    }
                });
                rangeInitEntries.filter(e => e.inside).forEach(e => {
                    t+=`        ${e.stateVar} = 1\n`;
                    if(e.enterActions.length > 0) {
                        t += buildLinkedSlaveActionINI(e.enterActions, '        ');
                    }
                });
                t+=`    endif\n`;
            }
            t+=`\n    ; Text Dialogue Runtime (paused while help is hidden)\n`;
            dialogueIni.present.forEach(line => { t+=`    ${line}\n`; });
            if(hasPersistentSheen) {
                t+=`        $persistent_phase = $persistent_phase + $persistent_speed\n`;
                t+=`        if $persistent_phase >= 1\n            $persistent_phase = $persistent_phase - 1\n        endif\n`;
            }

            if(groupList.length > 0) {
                t+=`\n    ; Group Global Animation\n`;
                groupList.forEach(group => {
                    const g = group.id;
                    const gid = getGroupRuntimeVarSuffix(g);
                    const offsetGid = getGroupRuntimeVarSuffix(groupOffsetOwnerById.get(g) || g);
                    t+=`    if $grp_anim_mode_${gid} > 0\n`;
                    t+=`        $grp_anim_phase_${gid} = $grp_anim_phase_${gid} + $grp_anim_speed_${gid}\n`;
                    t+=`        if $grp_anim_phase_${gid} >= 1\n            $grp_anim_phase_${gid} = $grp_anim_phase_${gid} - 1\n        endif\n`;
                    t+=`        $grp_anim_tx_${gid} = 0\n        $grp_anim_ty_${gid} = 0\n        $grp_anim_scale_${gid} = 1\n`;
                    t+= buildAnimationSampleAssignment(`$grp_anim_wave_${gid}`, `$grp_anim_phase_${gid}`, 'sine').replace(/^ {8}/gm, '        ');
                    t+=`        if $grp_anim_mode_${gid} == 1\n`;
                    t+= buildRuntimeEdgeDockCode({
                        resolvedEdgeVar: `$grp_anim_resolved_edge_${gid}`,
                        configEdgeVar: `$grp_anim_edge_${gid}`,
                        xExpr: `$grp_base_x_${gid} + $grp_off_x_${offsetGid}`,
                        yExpr: `$grp_base_y_${gid} + $grp_off_y_${offsetGid}`,
                        wExpr: `$grp_base_w_${gid}`,
                        hExpr: `$grp_base_h_${gid}`,
                        zoomExpr: `$zoom_global`,
                        touchTolVar: `$grp_anim_touch_tol_${gid}`,
                    triggerVar: `$grp_anim_trigger_${gid}`,
                    easeVar: `$grp_anim_ease_${gid}`,
                    revealVar: `$grp_anim_reveal_${gid}`,
                    progressVar: `$grp_anim_prog_${gid}`,
                    txVar: `$grp_anim_tx_${gid}`,
                    tyVar: `$grp_anim_ty_${gid}`,
                    dragLockExpr: '$is_dragging > 0'
                }).replace(/^ {8}/gm, '        ');
                    t+=`        else if $grp_anim_mode_${gid} == 2\n            $grp_anim_ty_${gid} = $grp_anim_wave_${gid} * $grp_anim_str_${gid}\n`;
                    t+=`        else if $grp_anim_mode_${gid} == 3\n            $grp_anim_tx_${gid} = $grp_anim_wave_${gid} * $grp_anim_str_${gid}\n`;
                    t+=`        else if $grp_anim_mode_${gid} == 4\n            $grp_anim_scale_${gid} = 1 + $grp_anim_wave_${gid} * $grp_anim_str_${gid} * 0.12\n`;
                    t+=`        endif\n`;
                    t+=`    endif\n`;
                });
            }

            t+=`\n    ; Component Global Animation\n`;
            components.forEach((m,i) => {
                if(componentExportMeta[i].animGroup || !componentExportUsage[i].componentGlobalAnimEnabled) return;
                const meta = componentExportMeta[i];
                const runtimeOffsetX = meta.offsetXExpr ? ` + ${meta.offsetXExpr}` : '';
                const runtimeOffsetY = meta.offsetYExpr ? ` + ${meta.offsetYExpr}` : '';
                t+=`    if $anim_global_mode_${i} > 0\n`;
                t+=`        $anim_global_phase_${i} = $anim_global_phase_${i} + $anim_global_speed_${i}\n`;
                t+=`        if $anim_global_phase_${i} >= 1\n            $anim_global_phase_${i} = $anim_global_phase_${i} - 1\n        endif\n`;
                t+=`        $anim_global_tx_${i} = 0\n        $anim_global_ty_${i} = 0\n        $anim_global_scale_${i} = 1\n`;
                t+= buildAnimationSampleAssignment(`$anim_global_wave_${i}`, `$anim_global_phase_${i}`, 'sine').replace(/^ {8}/gm, '        ');
                t+=`        if $anim_global_mode_${i} == 1\n`;
                t+= buildRuntimeEdgeDockCode({
                    resolvedEdgeVar: `$anim_global_resolved_edge_${i}`,
                    configEdgeVar: `$anim_global_edge_${i}`,
                    xExpr: `$base_x_${i} + $off_x_${i}${runtimeOffsetX}`,
                    yExpr: `$base_y_${i} + $off_y_${i}${runtimeOffsetY}`,
                    wExpr: `$base_w_${i}`,
                    hExpr: `$base_h_${i}`,
                    zoomExpr: `$zoom_global`,
                    touchTolVar: `$anim_global_touch_tol_${i}`,
                    triggerVar: `$anim_global_trigger_${i}`,
                    easeVar: `$anim_global_ease_${i}`,
                    revealVar: `$anim_global_reveal_${i}`,
                    progressVar: `$anim_global_prog_${i}`,
                    txVar: `$anim_global_tx_${i}`,
                    tyVar: `$anim_global_ty_${i}`,
                    dragLockExpr: '$is_dragging > 0'
                }).replace(/^ {8}/gm, '        ');
                t+=`        else if $anim_global_mode_${i} == 2\n            $anim_global_ty_${i} = $anim_global_wave_${i} * $anim_global_str_${i}\n`;
                t+=`        else if $anim_global_mode_${i} == 3\n            $anim_global_tx_${i} = $anim_global_wave_${i} * $anim_global_str_${i}\n`;
                t+=`        else if $anim_global_mode_${i} == 4\n            $anim_global_scale_${i} = 1 + $anim_global_wave_${i} * $anim_global_str_${i} * 0.12\n`;
                t+=`        endif\n`;
                t+=`    endif\n`;
            });

            t+=`\n    ; Local Animation\n`;
            components.forEach((m,i) => {
                if(!componentExportUsage[i].localAnimEnabled) return;
                const handleBobAssign = m.type === 'slider_v'
                    ? `        $anim_handle_dx_${i} = $temp * $anim_local_str_${i} * 0.05\n`
                    : `        $anim_handle_dy_${i} = $temp * $anim_local_str_${i} * 0.05\n`;
                t+=`    $anim_local_phase_${i} = $anim_local_phase_${i} + $anim_local_speed_${i}\n`;
                t+=`    if $anim_local_phase_${i} >= 1\n        $anim_local_phase_${i} = $anim_local_phase_${i} - 1\n    endif\n`;
                t+=`    $anim_local_dx_${i} = 0\n    $anim_local_dy_${i} = 0\n    $anim_local_scale_${i} = 1\n    $anim_local_alpha_${i} = 1\n    $anim_local_rot_${i} = 0\n    $anim_local_sheen_${i} = 0\n    $anim_handle_dx_${i} = 0\n    $anim_handle_dy_${i} = 0\n    $anim_handle_scale_${i} = 1\n    $anim_handle_alpha_${i} = 1\n    $anim_fill_alpha_${i} = 1\n    $anim_state_dx_${i} = 0\n    $anim_text_wave_${i} = 0\n    $anim_fx_boost_${i} = 0\n`;
                t+= buildAnimationSampleAssignment(`$temp`, `$anim_local_phase_${i}`, 'sine').replace(/^ {8}/gm, '    ');
                t+= buildAnimationSampleAssignment(`$d1`, `$anim_local_phase_${i}`, 'cosine').replace(/^ {8}/gm, '    ');
                t+=`    if $anim_local_mode_${i} == 1\n        $anim_local_scale_${i} = 1 + $temp * $anim_local_str_${i} * 0.08\n        $anim_local_alpha_${i} = 0.94 + $temp * $anim_local_str_${i} * 0.08\n        if $temp < 0\n            $d2 = -$temp\n        else\n            $d2 = $temp\n        endif\n        $anim_fx_boost_${i} = $d2 * $anim_local_str_${i} * 0.18\n`;
                t+=`    else if $anim_local_mode_${i} == 2\n        $anim_local_sheen_${i} = $anim_local_phase_${i}\n        $anim_fx_boost_${i} = 0.28 + $anim_local_str_${i} * 0.72\n`;
                t+=`    else if $anim_local_mode_${i} == 3\n${handleBobAssign}`;
                t+=`    else if $anim_local_mode_${i} == 4\n        if $temp < 0\n            $d2 = -$temp\n        else\n            $d2 = $temp\n        endif\n        $anim_fill_alpha_${i} = 0.84 + $d2 * $anim_local_str_${i} * 0.24\n        $anim_fx_boost_${i} = $d2 * $anim_local_str_${i} * 0.30\n`;
                t+=`    else if $anim_local_mode_${i} == 5\n        $anim_handle_dx_${i} = $temp * $anim_local_str_${i} * 0.04\n        $anim_handle_dy_${i} = $d1 * $anim_local_str_${i} * 0.04\n        $anim_local_rot_${i} = $temp * $anim_local_str_${i} * 0.10\n        if $temp < 0\n            $d2 = -$temp\n        else\n            $d2 = $temp\n        endif\n        $anim_handle_scale_${i} = 1 + $d2 * $anim_local_str_${i} * 0.06\n        $anim_fx_boost_${i} = 0.24 + $d2 * $anim_local_str_${i} * 0.46\n`;
                t+=`    else if $anim_local_mode_${i} == 6\n        $anim_handle_scale_${i} = 1 + $temp * $anim_local_str_${i} * 0.08\n        $anim_handle_alpha_${i} = 0.94 + $temp * $anim_local_str_${i} * 0.08\n        if $temp < 0\n            $d2 = -$temp\n        else\n            $d2 = $temp\n        endif\n        $anim_fx_boost_${i} = $d2 * $anim_local_str_${i} * 0.20\n`;
                t+=`    else if $anim_local_mode_${i} == 7\n        $anim_local_sheen_${i} = $anim_local_phase_${i}\n        $anim_fx_boost_${i} = 0.42 + $anim_local_str_${i} * 0.82\n`;
                t+=`    else if $anim_local_mode_${i} == 8\n        $anim_state_dx_${i} = $temp * $anim_local_str_${i} * 0.06\n        if $temp < 0\n            $d2 = -$temp\n        else\n            $d2 = $temp\n        endif\n        $anim_local_alpha_${i} = 0.94 + $d2 * $anim_local_str_${i} * 0.08\n        $anim_fx_boost_${i} = $d2 * $anim_local_str_${i} * 0.32\n`;
                t+=`    else if $anim_local_mode_${i} == 9\n        if $temp < 0\n            $d2 = -$temp\n        else\n            $d2 = $temp\n        endif\n        $anim_local_scale_${i} = 1 + $d2 * $anim_local_str_${i} * 0.14\n        $anim_fx_boost_${i} = $d2 * $anim_local_str_${i} * 0.24\n`;
                t+=`    else if $anim_local_mode_${i} == 10\n        $anim_text_wave_${i} = $temp * $anim_local_str_${i} * 0.04\n`;
                t+=`    else if $anim_local_mode_${i} == 11\n        if $temp < 0\n            $d2 = -$temp\n        else\n            $d2 = $temp\n        endif\n        $anim_local_alpha_${i} = 0.96 + $d2 * $anim_local_str_${i} * 0.06\n        $anim_fx_boost_${i} = 0.38 + $d2 * $anim_local_str_${i} * 0.82\n`;
                t+=`    else if $anim_local_mode_${i} == 12\n        $anim_local_scale_${i} = 1 + $temp * $anim_local_str_${i} * 0.08\n        $anim_local_alpha_${i} = 0.94 + $temp * $anim_local_str_${i} * 0.08\n        if $temp < 0\n            $d2 = -$temp\n        else\n            $d2 = $temp\n        endif\n        $anim_fx_boost_${i} = $d2 * $anim_local_str_${i} * 0.16\n`;
                t+=`    else if $anim_local_mode_${i} == 13\n        $anim_local_sheen_${i} = $anim_local_phase_${i}\n        $anim_fx_boost_${i} = 0.34 + $anim_local_str_${i} * 0.76\n`;
                t+=`    else if $anim_local_mode_${i} == 14\n        $anim_local_rot_${i} = $temp * $anim_local_str_${i} * 0.09\n        $anim_local_dy_${i} = $d1 * $anim_local_str_${i} * 0.03\n    endif\n`;
            });

            components.forEach((m,i) => {
                t+=`    $abs_w_${i} = $base_w_${i} * $zoom_global\n    $abs_h_${i} = $base_h_${i} * $zoom_global\n`;
                let baseX = `($base_x_${i} + $off_x_${i})`;
                let baseY = `($base_y_${i} + $off_y_${i})`;
                let widthExpr = `$base_w_${i}`;
                let heightExpr = `$base_h_${i}`;
                const meta = componentExportMeta[i];
                if(meta.offsetXExpr) {
                    baseX = `(${baseX} + ${meta.offsetXExpr})`;
                }
                if(meta.offsetYExpr) {
                    baseY = `(${baseY} + ${meta.offsetYExpr})`;
                }
                if(meta.animGroup) {
                    let gid = getGroupRuntimeVarSuffix(meta.animGroup.id);
                    baseX = `(${baseX} + $grp_anim_tx_${gid})`;
                    baseY = `(${baseY} + $grp_anim_ty_${gid})`;
                    widthExpr = `(${widthExpr} * $grp_anim_scale_${gid})`;
                    heightExpr = `(${heightExpr} * $grp_anim_scale_${gid})`;
                } else {
                    baseX = `(${baseX} + $anim_global_tx_${i})`;
                    baseY = `(${baseY} + $anim_global_ty_${i})`;
                    widthExpr = `($base_w_${i} * $anim_global_scale_${i})`;
                    heightExpr = `($base_h_${i} * $anim_global_scale_${i})`;
                }
                t+=`    $abs_w_${i} = ${widthExpr} * $zoom_global\n    $abs_h_${i} = ${heightExpr} * $zoom_global\n`;
                t+=`    $dx = (${baseX} + ${widthExpr}*0.5) - 0.5\n    $dy = (${baseY} + ${heightExpr}*0.5) - 0.5\n`;
                t+=`    $cx = 0.5 + $dx * $zoom_global\n    $cy = 0.5 + $dy * $zoom_global\n`;
                t+=`    $abs_x_${i} = $cx - $abs_w_${i}*0.5\n    $abs_y_${i} = $cy - $abs_h_${i}*0.5\n`;
                if((m.type === 'static' || m.type === 'sequence') && m.followCursor === true) {
                    const followOffsets = getFollowCursorOffsets(m);
                    const followXExpr = formatFollowCursorExpr('cursor_x', `$abs_w_${i}`, followOffsets.x);
                    const followYExpr = formatFollowCursorExpr('cursor_y', `$abs_h_${i}`, followOffsets.y);
                    t+=`    $abs_x_${i} = ${followXExpr}\n    $abs_y_${i} = ${followYExpr}\n`;
                }

                if(m.type !== 'text' && m.type !== 'static' && m.type !== 'toggle' && m.type !== 'sequence') {
                    t+=`    $sw = $hs_${i}*$zoom_global*$anim_handle_scale_${i}\n $sh = $hh_${i}*$zoom_global*$anim_handle_scale_${i}\n`;
                    let wT=`($abs_w_${i}-$sw)`, hT=`($abs_h_${i}-$sh)`;
                    if(m.type==='slider_h') {
                        t+=`    $dx = ($val_${i} * ${wT}) + $sw*0.5 - $abs_w_${i}*0.5 + $anim_handle_dx_${i} * $abs_w_${i}\n    $dy = $anim_handle_dy_${i} * $abs_h_${i}\n`;
                    } else if(m.type==='slider_v') {
                        t+=`    $dy = ((1-$val_${i}) * ${hT}) + $sh*0.5 - $abs_h_${i}*0.5 + $anim_handle_dy_${i} * $abs_h_${i}\n    $dx = $anim_handle_dx_${i} * $abs_w_${i}\n`;
                    } else {
                        t+=`    $dx = (($val_${i}_x+1)*0.5 * ${wT}) + $sw*0.5 - $abs_w_${i}*0.5 + $anim_handle_dx_${i} * $abs_w_${i}\n`;
                        t+=`    $dy = ((1-$val_${i}_y)*0.5 * ${hT}) + $sh*0.5 - $abs_h_${i}*0.5 + $anim_handle_dy_${i} * $abs_h_${i}\n`;
                    }
                    t+=`    $rx = $dx * $aspect\n    $ry = $dy\n`;
                    t+=`    $r_hdl_${i}_x = $abs_x_${i} + $abs_w_${i}*0.5 + ($rx * $cos_${i} - $ry * $sin_${i}) / $aspect - $sw*0.5\n`;
                    t+=`    $r_hdl_${i}_y = $abs_y_${i} + $abs_h_${i}*0.5 + ($rx * $sin_${i} + $ry * $cos_${i}) - $sh*0.5\n`;
                }
            });

            components.forEach((m, i) => {
                if(m.type !== 'text' || !m.textHoverEffect) return;
                const visCond = componentExportMeta[i].visibilityExpr ? `(${componentExportMeta[i].visibilityExpr})` : '';
                t+=`    $text_hover_${i} = 0\n`;
                t+=`    if $help == 1\n`;
                if(visCond) t+=`        if ${visCond}\n`;
                t+=`        $cx = $abs_x_${i} + $abs_w_${i}*0.5\n        $cy = $abs_y_${i} + $abs_h_${i}*0.5\n`;
                t+=`        $dx = (cursor_x - $cx) * $aspect\n        $dy = cursor_y - $cy\n`;
                t+=`        $rx = ($dx * $cos_${i} + $dy * $sin_${i}) / $aspect\n`;
                t+=`        $ry = $dy * $cos_${i} - $dx * $sin_${i}\n`;
                t+=`        if $rx > -$abs_w_${i}*0.54 && $rx < $abs_w_${i}*0.54 && $ry > -$abs_h_${i}*0.54 && $ry < $abs_h_${i}*0.54\n`;
                t+=`            $text_hover_${i} = 1\n`;
                t+=`            $abs_w_${i} = $abs_w_${i} * 1.08\n            $abs_h_${i} = $abs_h_${i} * 1.08\n`;
                t+=`            $abs_x_${i} = $cx - $abs_w_${i}*0.5\n            $abs_y_${i} = $cy - $abs_h_${i}*0.5\n`;
                t+=`        endif\n`;
                if(visCond) t+=`        endif\n`;
                t+=`    endif\n`;
            });


            t+=`\n    if $mouse_clicked == 0\n        $is_dragging = 0\n        $drag_action = 0\n    endif\n\n    if $mouse_clicked == 1 && $is_dragging == 0\n`;
            interactionOrder.forEach((i) => {
                let m=components[i];
                // 跟随鼠标的静态/序列帧组件始终位于指针正下方，不参与命中与拖拽，避免吞掉其他组件的点击。
                if((m.type === 'static' || m.type === 'sequence') && m.followCursor === true) return;
                const meta = componentExportMeta[i];
                const visCond = meta.visibilityExpr ? `(${meta.visibilityExpr})` : '';
                t+=`        if $help == 1\n`;
                if(visCond) t+=`            if ${visCond}\n`;
                t+=`        $cx = $abs_x_${i} + $abs_w_${i}*0.5\n $cy = $abs_y_${i} + $abs_h_${i}*0.5\n`;
                t+=`        $dx = (${cursorXExpr} - $cx) * $aspect\n $dy = (${cursorYExpr} - $cy)\n`;
                t+=`        $rx = ($dx * $cos_${i} + $dy * $sin_${i}) / $aspect\n`;
                t+=`        $ry = $dy * $cos_${i} - $dx * $sin_${i}\n`;
                // 模型区域拖拽已改为独立的锁存绑定块（见下方“模型区域拖拽绑定”段），不再在鼠标命中循环内认领。
                let mw=`$abs_w_${i}*0.5`, mh=`$abs_h_${i}*0.5`; 
                let outerHitCheck = `$rx > -${mw} && $rx < ${mw} && $ry > -${mh} && $ry < ${mh}`;
                let hasControl = (m.type !== 'static' && m.type !== 'text' && m.type !== 'sequence' && m.type !== 'accum') || (m.type === 'text' && (!!getTextClickVar(m) || dialogueIni.clickCodeByComponentId.has(m.id)));
                if (hasControl) {
                    let innerCheck = "";
                    if (m.type.includes('slider') || m.type === 'joystick') {
                        let hSzX = `($hs_${i}*$zoom_global*0.5)`;
                        let hSzY = `($hh_${i}*$zoom_global*0.5)`;
                        if (m.type === 'slider_h') innerCheck = `($ry > -${hSzY} && $ry < ${hSzY})`;
                        else if (m.type === 'slider_v') innerCheck = `($rx > -${hSzX} && $rx < ${hSzX})`;
                        else if (m.type === 'joystick') innerCheck = `($rx > -$abs_w_${i}*0.3333 && $rx < $abs_w_${i}*0.3333 && $ry > -$abs_h_${i}*0.3333 && $ry < $abs_h_${i}*0.3333)`;
                    } else if (m.type === 'toggle' || m.type === 'text') innerCheck = `1`; 
                    
                    t+=`        if ${outerHitCheck}\n`;
                    t+=`            if $is_dragging == 0\n`;
                    t+=`            $is_dragging = ${i+1}\n $drag_dx = $rx\n $drag_dy = $ry\n`;
                   t+=`            if ${innerCheck}\n $drag_action = 2\n`; 
                   t += buildClickActionCode(m, i, '                ');
                    t += buildAccumClaimInitCode(m, i, '                ');
                   t+=`            else\n $drag_action = 1\n endif\n`; 
                    t+=`            endif\n`;
                    t+=`        endif\n`; 
                } else {
                    const staticDragCheck = outerHitCheck;
                    t+=`        if ${staticDragCheck}\n`;
                    t+=`            if $is_dragging == 0\n`;
                    t+=`            $is_dragging = ${i+1}\n $drag_dx = $rx\n $drag_dy = $ry\n $drag_action = 1\n`;
                    t+=`            endif\n`;
                    t+=`        endif\n`; 
                }
                if(visCond) t+=`            endif\n`;
                t+=`        endif\n`; 
            });
            t+=`    endif\n\n`;

            // 模型区域拖拽绑定（独立块）：不依赖面板开关（$help），拖拽侧运行模式必须为 1（仅命中）。
            // Alt+左键 / Alt+X 按下且命中指定区域时锁存绑定本组件手柄；绑定期间光标移出区域不中断；
            // 松开左键 / X 键 / Alt 后 gate 不成立 → 锁存清零，手柄照常回弹/吸附。
            interactionOrder.forEach((i) => {
                let m = components[i];
                if(!componentUsesZoneDrag(m)) return;
                const ns = getZoneDragNamespace(m);
                const nsSuffix = ns ? `_${ns}` : '';
                const latchVar = getZoneDragLatchVarName(m, i);
                t+=`    ; 模型区域拖拽绑定: comp ${i + 1} (${getZoneDragVarName(m)} == ${getZoneDragZoneId(m)})\n`;
                t+=`    if $ssmtdrag_drag_enabled${nsSuffix} == 1 && $ssmtdrag_modifier_down${nsSuffix} == 1 && ($ssmtdrag_lmb_down${nsSuffix} == 1 || $ssmtdrag_x_down${nsSuffix} == 1)\n`;
                t+=`        if ${latchVar} == 0\n`;
                t+=`            if ${getZoneDragVarName(m)} == ${getZoneDragZoneId(m)}\n`;
                t+=`                $cx = $abs_x_${i} + $abs_w_${i}*0.5\n                $cy = $abs_y_${i} + $abs_h_${i}*0.5\n`;
                t+=`                $dx = (${cursorXExpr} - $cx) * $aspect\n                $dy = (${cursorYExpr} - $cy)\n`;
                t+=`                $rx = ($dx * $cos_${i} + $dy * $sin_${i}) / $aspect\n`;
                t+=`                $ry = $dy * $cos_${i} - $dx * $sin_${i}\n`;
                t+=`                $drag_dx = $rx\n                $drag_dy = $ry\n`;
                t += buildAccumClaimInitCode(m, i, '                ');
                if(m.type === 'joystick') {
                    t+=`                $zgrab_rx = $rx - $val_${i}_x * ($abs_w_${i}-$hs_${i}*$zoom_global) * 0.5\n`;
                    t+=`                $zgrab_ry = $ry + $val_${i}_y * ($abs_h_${i}-$hh_${i}*$zoom_global) * 0.5\n`;
                } else if(m.type === 'slider_v') {
                    t+=`                $zgrab_rx = 0\n`;
                    t+=`                $zgrab_ry = $ry - ($abs_h_${i}-$hh_${i}*$zoom_global) * (0.5 - $val_${i})\n`;
                } else {
                    t+=`                $zgrab_rx = $rx - ($abs_w_${i}-$hs_${i}*$zoom_global) * ($val_${i} - 0.5)\n`;
                    t+=`                $zgrab_ry = 0\n`;
                }
                t+=`                ${latchVar} = 1\n`;
                t+=`            endif\n`;
                t+=`        endif\n`;
                t+=`        if ${latchVar} == 1\n`;
                t+=`            $is_dragging = ${i+1}\n            $drag_action = 3\n`;
                t+=`        endif\n`;
                t+=`    else\n`;
                t+=`        ${latchVar} = 0\n`;
                t+=`    endif\n`;
            });

            t+=`    if $is_dragging > 0\n`;
            t+=`        $cursor_delta_x = (${cursorXExpr} - $prev_cursor_x) / $zoom_global\n`;
            t+=`        $cursor_delta_y = (${cursorYExpr} - $prev_cursor_y) / $zoom_global\n`;
            components.forEach((m,i) => {
                const meta = componentExportMeta[i];
                const usage = componentExportUsage[i];
                let wT=`($abs_w_${i}-$sw)`, hT=`($abs_h_${i}-$sh)`;
                t+=`        if $is_dragging==${i+1}\n`;
                t+=`            if $drag_action == 1\n              if $help == 1\n`;
                if (meta.topmostGroup) {
                    let gid = getGroupRuntimeVarSuffix(meta.topmostGroup.id);
                    t+=`                $grp_off_x_${gid} = $grp_off_x_${gid} + $cursor_delta_x\n`;
                    t+=`                $grp_off_y_${gid} = $grp_off_y_${gid} + $cursor_delta_y\n`;
                    const animGroupProfile = meta.animGroup ? animatedGroups.get(meta.animGroup.id) : null;
                    if(animGroupProfile && animGroupProfile.modeCode === 1) {
                        const animGid = getGroupRuntimeVarSuffix(meta.animGroup.id);
                        t+=buildRuntimeEdgeSnapCode({
                            modeVar: `$grp_anim_mode_${animGid}`,
                            configEdgeVar: `$grp_anim_edge_${animGid}`,
                            offsetXVar: `$grp_off_x_${gid}`,
                            offsetYVar: `$grp_off_y_${gid}`,
                            xExpr: `$grp_base_x_${animGid} + $grp_off_x_${gid}`,
                            yExpr: `$grp_base_y_${animGid} + $grp_off_y_${gid}`,
                            wExpr: `$grp_base_w_${animGid}`,
                            hExpr: `$grp_base_h_${animGid}`,
                            triggerVar: `$grp_anim_trigger_${animGid}`
                        });
                    } else if(usage.componentGlobalAnimProfile && usage.componentGlobalAnimProfile.modeCode === 1) {
                        t+=buildRuntimeEdgeSnapCode({
                            modeVar: `$anim_global_mode_${i}`,
                            configEdgeVar: `$anim_global_edge_${i}`,
                            offsetXVar: `$grp_off_x_${gid}`,
                            offsetYVar: `$grp_off_y_${gid}`,
                            xExpr: `$base_x_${i} + $off_x_${i} + $grp_off_x_${gid}`,
                            yExpr: `$base_y_${i} + $off_y_${i} + $grp_off_y_${gid}`,
                            wExpr: `$base_w_${i}`,
                            hExpr: `$base_h_${i}`,
                            triggerVar: `$anim_global_trigger_${i}`
                        });
                    }
                } else {
                    t+=`                $off_x_${i} = $off_x_${i} + $cursor_delta_x\n`;
                    t+=`                $off_y_${i} = $off_y_${i} + $cursor_delta_y\n`;
                    if(usage.componentGlobalAnimProfile && usage.componentGlobalAnimProfile.modeCode === 1) {
                        t+=buildRuntimeEdgeSnapCode({
                            modeVar: `$anim_global_mode_${i}`,
                            configEdgeVar: `$anim_global_edge_${i}`,
                            offsetXVar: `$off_x_${i}`,
                            offsetYVar: `$off_y_${i}`,
                            xExpr: `$base_x_${i} + $off_x_${i}`,
                            yExpr: `$base_y_${i} + $off_y_${i}`,
                            wExpr: `$base_w_${i}`,
                            hExpr: `$base_h_${i}`,
                            triggerVar: `$anim_global_trigger_${i}`
                        });
                    }
                }
                t+=`              endif\n            endif\n`;
                t+=`            if $drag_action >= 2\n`;
                
                if(m.type!=='toggle' && m.type!=='static' && m.type!=='text' && m.type!=='sequence' && m.type!=='accum') {
                    t+=`                $sw = $hs_${i}*$zoom_global\n                $sh = $hh_${i}*$zoom_global\n`;
                    t+=`                $cx = $abs_x_${i} + $abs_w_${i}*0.5\n $cy = $abs_y_${i} + $abs_h_${i}*0.5\n`;
                    t+=`                $dx = (${cursorXExpr} - $cx) * $aspect\n $dy = (${cursorYExpr} - $cy)\n`;
                    t+=`                $rx = ($dx * $cos_${i} + $dy * $sin_${i}) / $aspect\n`;
                    t+=`                $ry = $dy * $cos_${i} - $dx * $sin_${i}\n`;
                    if(componentUsesZoneDrag(m)) {
                        // 区域认领的相对拖动(action 3)：减去皮拉偏移，手柄从当前值按位移相对移动
                        t+=`                if $drag_action == 3\n                    $rx = $rx - $zgrab_rx\n                    $ry = $ry - $zgrab_ry\n                endif\n`;
                    }
                    
                    if(m.type==='slider_h') {
                        t+=`                $val_${i} = ($rx + (${wT}) * 0.5) / ${wT}\n`;
                        t+=`                if $val_${i} < 0\n $val_${i} = 0\n endif\n if $val_${i} > 1\n $val_${i} = 1\n endif\n`;
                    } else if(m.type==='slider_v') {
                        t+=`                $val_${i} = 1 - (($ry + (${hT}) * 0.5) / ${hT})\n`;
                        t+=`                if $val_${i} < 0\n $val_${i} = 0\n endif\n if $val_${i} > 1\n $val_${i} = 1\n endif\n`;
                    } else { 
                        if(m.physicsProfile === 'breast') {
                            t+=`                if $phys_mode_${i} == 1\n`;
                            t+=`                    $drive_${i}_x = ($rx / (${wT}*0.5))\n`; 
                            t+=`                    if $drive_${i}_x < -1\n $drive_${i}_x = -1\n endif\n if $drive_${i}_x > 1\n $drive_${i}_x = 1\n endif\n`;
                            t+=`                    $drive_${i}_y = -($ry / (${hT}*0.5))\n`;
                            t+=`                    if $drive_${i}_y < -1\n $drive_${i}_y = -1\n endif\n if $drive_${i}_y > 1\n $drive_${i}_y = 1\n endif\n`;
                            t+=`                    $force = ($drive_${i}_x - $val_${i}_x) * (($spring_k_${i} * 2.20) + 0.18)\n`;
                            t+=`                    $vel_${i}_x = ($vel_${i}_x + $force) * (($spring_d_${i} * 0.52) + 0.28)\n`;
                            t+=`                    $val_${i}_x = $val_${i}_x + $vel_${i}_x\n`;
                            t+=`                    $force = ($drive_${i}_y - $val_${i}_y) * (($spring_k_${i} * 2.85) + 0.24)\n`;
                            t+=`                    $temp = $drive_${i}_y - $val_${i}_y\n`;
                            t+=`                    if $temp < 0\n                        $temp = -$temp\n                    endif\n`;
                            t+=`                    $force = $force + $temp * (($spring_k_${i} * 0.35) + 0.05)\n`;
                            t+=`                    if $gravity_${i} != 0\n                        $force = $force - $gravity_${i} * 0.22\n                    endif\n`;
                            t+=`                    $vel_${i}_y = ($vel_${i}_y + $force) * (($spring_d_${i} * 0.48) + 0.26)\n`;
                            t+=`                    $val_${i}_y = $val_${i}_y + $vel_${i}_y\n`;
                            t+=`                else\n`;
                            t+=`                    $val_${i}_x = ($rx / (${wT}*0.5))\n`; 
                            t+=`                    if $val_${i}_x < -1\n $val_${i}_x = -1\n endif\n if $val_${i}_x > 1\n $val_${i}_x = 1\n endif\n`;
                            t+=`                    $val_${i}_y = -($ry / (${hT}*0.5))\n`;
                            t+=`                    if $val_${i}_y < -1\n $val_${i}_y = -1\n endif\n if $val_${i}_y > 1\n $val_${i}_y = 1\n endif\n`;
                            t+=`                endif\n`;
                       } else {
                           t+=`                $val_${i}_x = ($rx / (${wT}*0.5))\n`; 
                           t+=`                if $val_${i}_x < -1\n $val_${i}_x = -1\n endif\n if $val_${i}_x > 1\n $val_${i}_x = 1\n endif\n`;
                           t+=`                $val_${i}_y = -($ry / (${hT}*0.5))\n`;
                           t+=`                if $val_${i}_y < -1\n $val_${i}_y = -1\n endif\n if $val_${i}_y > 1\n $val_${i}_y = 1\n endif\n`;
                       }
                   }
               }
                t += buildAccumDragDeltaCode(m, i, '                ');
               t+=`            endif\n        endif\n`;
            });
            t+=`    endif\n`; // close $is_dragging > 0

            // 格子模式吸附处理（始终运行，但每个组件检测自己的 $is_dragging）
            components.forEach((m,i) => {
                if(m.type!=='slider_h' && m.type!=='slider_v') return;
                if(m.paramMode !== '3') return;
                
                let gridSteps = m.gridSteps || 3;
                let interval = 1.0 / (gridSteps - 1);
                
                t+=`    ; 格子模式 (${gridSteps} 档)\n`;
                t+=`    if $is_dragging != ${i+1}\n`;
                // 直接生成嵌套 if/else，不使用 else if（避免 expandElseIfChains 的缩进 bug）
                for(let s = 0; s < gridSteps; s++) {
                    let pos = s * interval;
                    if(s < gridSteps - 1) {
                        let nextPos = (s + 1) * interval;
                        t+=`        if $val_${i} < ${nextPos.toFixed(6)}\n`;
                        t+=`            $val_${i} = ${pos.toFixed(6)}\n`;
                        t+=`        else\n`;
                    } else {
                        t+=`        else\n`;
                        t+=`            $val_${i} = ${pos.toFixed(6)}\n`;
                    }
                }
                // 关闭所有嵌套的 if
                for(let s = 0; s < gridSteps - 1; s++) {
                    t+=`        endif\n`;
                }
                t+=`    endif\n`;
            });
            
            // ========== 从属联动覆写 (在物理计算之前执行, 确保 bounce 数值生效) ==========
            // 遍历所有组件的 linkedSlaves, 为每对联动生成 if/else 代码块
            // 覆盖所有组合: 滑块→滑块, 滑块→摇杆, 摇杆→滑块, 摇杆→摇杆
            const linkedSlaveCodeBlocks = [];
            // 区间动作改为边沿触发：先收集所有“进入/离开”转换，再统一输出两阶段区块，
            // 避免多个区间/联动块共享同一动作变量时互相覆盖（后写的块覆盖先写的块）。
            const rangeLeaveTransitions = [];
            const rangeEnterTransitions = [];
            components.forEach((srcComp, srcIdx) => {
                const slaves = Array.isArray(srcComp.linkedSlaves) ? srcComp.linkedSlaves : [];
                slaves.forEach((link, linkIdx) => {
                    if(!link || !link.enabled) return;
                    const tgtComp = link.targetId ? components.find(c => c.id === link.targetId) : null;
                    const tgtIdx = tgtComp ? components.indexOf(tgtComp) : -1;
                    const hasMappingTarget = !!tgtComp && tgtIdx >= 0;
                    const effectiveRegionMode = getLinkedSlaveEffectiveRegionMode(link, srcComp, tgtComp);
                    const isSrcJoystick = srcComp.type === 'joystick';
                    const isSrcDir = isSrcJoystick && srcComp.paramMode === '4';
                    const isRectMode = isSrcJoystick && effectiveRegionMode === 'rect';
                    const targetDragGuard = hasMappingTarget ? `$is_dragging != ${tgtIdx + 1}` : '1';
                    // 区间配置: srcMin/srcMax 定义源值触发范围
                    const srcMin = link.srcMin ?? 0;
                    const srcMax = link.srcMax ?? 0.5;
                    const range = srcMax - srcMin;
                    const overflow = link.overflow || 'reset';
                    const splitSide = link.splitSide || 'both';
                    const enterActions = getLinkedSlaveConfiguredActions(link, 'enter');
                    const leaveActions = getLinkedSlaveConfiguredActions(link, 'leave');
                    const hasRangeActions = enterActions.length > 0 || leaveActions.length > 0;
                    const rangeStateVar = hasRangeActions ? `$linked_range_${srcIdx}_${linkIdx}` : null;
                    if(!hasMappingTarget && !hasRangeActions) return;
                    if(!isRectMode && range <= 0) return;
                    
                    // ---- 确定源值变量名 $srcVar ----
                    // 根据源组件类型和轴选择生成对应的 INI 变量引用
                    let srcVar;
                    let srcVarPrelude = '';
                    if(isSrcJoystick && !isSrcDir) {
                        const axis = (link.joyAxis !== undefined && link.joyAxis !== null && link.joyAxis !== '') ? link.joyAxis : 'both';
                        if(axis === 'x') srcVar = `$val_${srcIdx}_x`;
                        else if(axis === 'y') srcVar = `$val_${srcIdx}_y`;
                        else {
                            srcVar = `$src_scalar_${srcIdx}_${linkIdx}`;
                            srcVarPrelude += `    if $val_${srcIdx}_x < 0\n        $src_abs_x_${srcIdx}_${linkIdx} = -$val_${srcIdx}_x\n    else\n        $src_abs_x_${srcIdx}_${linkIdx} = $val_${srcIdx}_x\n    endif\n`;
                            srcVarPrelude += `    if $val_${srcIdx}_y < 0\n        $src_abs_y_${srcIdx}_${linkIdx} = -$val_${srcIdx}_y\n    else\n        $src_abs_y_${srcIdx}_${linkIdx} = $val_${srcIdx}_y\n    endif\n`;
                            srcVarPrelude += `    if $src_abs_x_${srcIdx}_${linkIdx} > $src_abs_y_${srcIdx}_${linkIdx}\n        ${srcVar} = $src_abs_x_${srcIdx}_${linkIdx}\n    else\n        ${srcVar} = $src_abs_y_${srcIdx}_${linkIdx}\n    endif\n`;
                        }
                    } else if(!isSrcJoystick) {
                        srcVar = `$val_${srcIdx}`;
                    } else if(isSrcDir) {
                        // 方向源：非方向目标时使用合成 X/Y 或方向值
                        const tgtDirCheck = hasMappingTarget && tgtComp && tgtComp.type === 'joystick' && tgtComp.paramMode === '4';
                        if(tgtDirCheck) {
                            // perDirection 内逐方向设定 srcVar
                        } else {
                            const axis = (link.joyAxis !== undefined && link.joyAxis !== null && link.joyAxis !== '') ? link.joyAxis : 'x';
                            if(axis !== 'x' && axis !== 'y') {
                                // 方向索引：使用 $joy_dir_#_d
                                const dirIdx = parseInt(axis, 10) || 0;
                                srcVar = `$joy_dir_${srcIdx}_${dirIdx}`;
                            } else if(axis === 'y') {
                                srcVar = `$val_${srcIdx}_y`;
                            } else {
                                srcVar = `$val_${srcIdx}_x`;
                            }
                        }
                    }
                    // For directional mode with perDirection, srcVar is set per-direction
                    if(!srcVar && !isSrcDir) return;
                    
                    // 确定目标变量名
                    let tgtIsJoystick = hasMappingTarget && tgtComp.type === 'joystick';
                    const tgtIsDir = hasMappingTarget && tgtIsJoystick && tgtComp.paramMode === '4';
                    let tgtVar;
                    const tgtAxis = (link.joyTargetAxis !== undefined && link.joyTargetAxis !== null && link.joyTargetAxis !== '') ? link.joyTargetAxis : 'x';
                    if(tgtIsDir) {
                        // 方向模式摇杆目标：写入对应方向值 $joy_dir_#_d
                        const dirIdx = parseInt(tgtAxis, 10) || 0;
                        tgtVar = `$joy_dir_${tgtIdx}_${dirIdx}`;
                    } else if(tgtIsJoystick) {
                        tgtVar = `$val_${tgtIdx}_${tgtAxis}`;
                    } else if(hasMappingTarget) {
                        tgtVar = `$val_${tgtIdx}`;
                    }
                    
                    const isTgtBidir = hasMappingTarget && !tgtIsJoystick && tgtComp.paramMode === '2';
                    const isTgtGrid = hasMappingTarget && !tgtIsJoystick && isSliderGridMode(tgtComp);
                    const gridSteps = isTgtGrid ? Math.max(2, tgtComp.gridSteps || 3) : 0;
                    const tgtHasPhysics = hasMappingTarget && componentExportUsage[tgtIdx] && componentExportUsage[tgtIdx].physicsEnabled;
                    const restVar = tgtIsJoystick ? `$rest_${tgtIdx}_${tgtAxis}` : `$rest_${tgtIdx}`;
                    
                    // 四边形区域模式: 4 点凸四边形内判定
                    if(isRectMode) {
                        const pts = getLinkedSlaveRegionPoints(link);
                        const sx = `$val_${srcIdx}_x`, sy = `$val_${srcIdx}_y`;
                        
                        // 生成 4 条边的叉积检测: cross(edge_i, P - v_i)
                        const crossExprs = [];
                        for(let i = 0; i < 4; i++) {
                            const j = (i + 1) % 4;
                            const ex = pts[j].x - pts[i].x, ey = pts[j].y - pts[i].y;
                            crossExprs.push(
                                `(${ex.toFixed(6)}) * (${sy} - ${pts[i].y.toFixed(6)}) - (${ey.toFixed(6)}) * (${sx} - ${pts[i].x.toFixed(6)})`
                            );
                        }
                        const qVars = crossExprs.map((_, i) => `$quad_cross_${srcIdx}_${linkIdx}_${i}`);
                        
                        // 凸四边形内判定: 所有叉积同号
                        const insideExpr = `(${qVars.map(v => `${v} >= 0`).join(' && ')}) || (${qVars.map(v => `${v} <= 0`).join(' && ')})`;
                        const insideGuardExpr = `(${insideExpr})`;
                        
                        let block = `    ; Linked Quad: comp${srcIdx}${hasMappingTarget ? ` -> comp${tgtIdx}` : ' -> actions-only'}`;
                        block += ` P0(${pts[0].x.toFixed(2)},${pts[0].y.toFixed(2)}) P1(${pts[1].x.toFixed(2)},${pts[1].y.toFixed(2)}) P2(${pts[2].x.toFixed(2)},${pts[2].y.toFixed(2)}) P3(${pts[3].x.toFixed(2)},${pts[3].y.toFixed(2)}) overflow=${overflow}\n`;
                        
                        // 计算叉积值
                        for(let i = 0; i < 4; i++) {
                            block += `    ${qVars[i]} = ${crossExprs[i]}\n`;
                        }
                        
                        if(hasRangeActions) {
                            // 边沿触发：进入/离开转换统一在下方两阶段区块中应用（离开优先、进入最后）
                            const actionLabel = `Linked Quad comp${srcIdx}${hasMappingTarget ? ` -> comp${tgtIdx}` : ''}`;
                            rangeLeaveTransitions.push({ label: actionLabel, insideExpr, stateVar: rangeStateVar, actions: leaveActions });
                            rangeEnterTransitions.push({ label: actionLabel, insideExpr, stateVar: rangeStateVar, actions: enterActions });
                        }
                        if(!hasMappingTarget) {
                            linkedSlaveCodeBlocks.push(block);
                            return;
                        }
                        block += `    if ${targetDragGuard}\n`;
                        
                        let gateExpr = insideExpr;
                        const postEntry = computeDirectionalLinkedPost(link, srcComp, tgtComp);
                        if(postEntry) {
                            const postCX = postEntry.posX.toFixed(6);
                            const postCY = postEntry.posY.toFixed(6);
                            const postR2 = (postEntry.radius * postEntry.radius).toFixed(6);
                            block += `    ; Quad collision post (${postCX}, ${postCY}) r=${postEntry.radius.toFixed(4)}\n`;
                            block += `    if $linked_post_${tgtIdx} == 0\n`;
                            block += `        $d3 = (${sx} - ${postCX})*(${sx} - ${postCX}) + (${sy} - ${postCY})*(${sy} - ${postCY})\n`;
                            block += `        if ${insideGuardExpr} && $d3 <= ${postR2}\n`;
                            block += `            $linked_post_${tgtIdx} = 1\n`;
                            block += `        endif\n`;
                            block += `    endif\n`;
                            block += `    if ${insideGuardExpr}\n`;
                            block += `    else\n`;
                            block += `        if $linked_post_${tgtIdx} != 0\n`;
                            block += `            $linked_post_${tgtIdx} = 0\n`;
                            if(tgtHasPhysics) {
                                // 曾激活后离开四边形：归零回弹，避免 $rest 停留在最后映射值；
                                // 复位联动占用闩锁 $linked_active（物理自动驱动条件 = $auto==1 && $linked_active==0），
                                // 由 $auto 真实值决定是否恢复自动动画——不直接写 $auto，
                                // 避免覆盖绑定开关并触发 Sync Bindings follow 反向拉值（A1 修复）
                                block += `            $rest_${tgtIdx}_x = 0\n            $rest_${tgtIdx}_y = 0\n`;
                                block += `            $linked_active_${tgtIdx} = 0\n`;
                            } else {
                                block += `            $val_${tgtIdx}_x = 0\n            $val_${tgtIdx}_y = 0\n`;
                            }
                            block += `        endif\n`;
                            block += `    endif\n`;
                            gateExpr = `${insideGuardExpr} && $linked_post_${tgtIdx} != 0`;
                        }

                        const p00 = pts[0];
                        const p10 = pts[1];
                        const p11 = pts[2];
                        const p01 = pts[3];
                        block += `    $rtemp_x = 0.500000\n`;
                        block += `    $rtemp_y = 0.500000\n`;
                        block += `    if ${gateExpr}\n`;
                        for(let iter = 0; iter < 8; iter++) {
                            block += `        $quad_fx_${srcIdx}_${linkIdx} = (1-$rtemp_x)*(1-$rtemp_y)*${p00.x.toFixed(6)} + $rtemp_x*(1-$rtemp_y)*${p10.x.toFixed(6)} + $rtemp_x*$rtemp_y*${p11.x.toFixed(6)} + (1-$rtemp_x)*$rtemp_y*${p01.x.toFixed(6)} - (${sx})\n`;
                            block += `        $quad_fy_${srcIdx}_${linkIdx} = (1-$rtemp_x)*(1-$rtemp_y)*${p00.y.toFixed(6)} + $rtemp_x*(1-$rtemp_y)*${p10.y.toFixed(6)} + $rtemp_x*$rtemp_y*${p11.y.toFixed(6)} + (1-$rtemp_x)*$rtemp_y*${p01.y.toFixed(6)} - (${sy})\n`;
                            block += `        $quad_j00_${srcIdx}_${linkIdx} = (1-$rtemp_y)*${(p10.x - p00.x).toFixed(6)} + $rtemp_y*${(p11.x - p01.x).toFixed(6)}\n`;
                            block += `        $quad_j01_${srcIdx}_${linkIdx} = (1-$rtemp_x)*${(p01.x - p00.x).toFixed(6)} + $rtemp_x*${(p11.x - p10.x).toFixed(6)}\n`;
                            block += `        $quad_j10_${srcIdx}_${linkIdx} = (1-$rtemp_y)*${(p10.y - p00.y).toFixed(6)} + $rtemp_y*${(p11.y - p01.y).toFixed(6)}\n`;
                            block += `        $quad_j11_${srcIdx}_${linkIdx} = (1-$rtemp_x)*${(p01.y - p00.y).toFixed(6)} + $rtemp_x*${(p11.y - p10.y).toFixed(6)}\n`;
                            block += `        $quad_det_${srcIdx}_${linkIdx} = $quad_j00_${srcIdx}_${linkIdx} * $quad_j11_${srcIdx}_${linkIdx} - $quad_j01_${srcIdx}_${linkIdx} * $quad_j10_${srcIdx}_${linkIdx}\n`;
                            block += `        if $quad_det_${srcIdx}_${linkIdx} > 0.000001 || $quad_det_${srcIdx}_${linkIdx} < -0.000001\n`;
                            block += `            $rtemp_x = $rtemp_x - ($quad_j11_${srcIdx}_${linkIdx} * $quad_fx_${srcIdx}_${linkIdx} - $quad_j01_${srcIdx}_${linkIdx} * $quad_fy_${srcIdx}_${linkIdx}) / $quad_det_${srcIdx}_${linkIdx}\n`;
                            block += `            $rtemp_y = $rtemp_y - ($quad_j00_${srcIdx}_${linkIdx} * $quad_fy_${srcIdx}_${linkIdx} - $quad_j10_${srcIdx}_${linkIdx} * $quad_fx_${srcIdx}_${linkIdx}) / $quad_det_${srcIdx}_${linkIdx}\n`;
                            block += `        endif\n`;
                            block += `        if $rtemp_x < 0\n            $rtemp_x = 0\n        endif\n`;
                            block += `        if $rtemp_x > 1\n            $rtemp_x = 1\n        endif\n`;
                            block += `        if $rtemp_y < 0\n            $rtemp_y = 0\n        endif\n`;
                            block += `        if $rtemp_y > 1\n            $rtemp_y = 1\n        endif\n`;
                        }

                        if(tgtIsJoystick) {
                            block += `        $val_${tgtIdx}_x = $rtemp_x * 2 - 1\n`;
                            block += `        $val_${tgtIdx}_y = 1 - $rtemp_y * 2\n`;
                            if(tgtHasPhysics) {
                                block += `        $rest_${tgtIdx}_x = $val_${tgtIdx}_x\n        $rest_${tgtIdx}_y = $val_${tgtIdx}_y\n        $linked_active_${tgtIdx} = 1\n`;
                            }
                        } else {
                            const isTgtBidir = hasMappingTarget && tgtComp.paramMode === '2';
                            const isTgtGrid = hasMappingTarget && isSliderGridMode(tgtComp);
                            const gridSteps = isTgtGrid ? Math.max(2, tgtComp.gridSteps || 3) : 0;
                            let mappedExpr = '$rtemp_x';
                            if(isTgtGrid) {
                                const interval = 1.0 / (gridSteps - 1);
                                block += `        ; Grid snap (${gridSteps} steps)\n`;
                                for(let s = 0; s < gridSteps; s++) {
                                    const pos = s * interval;
                                    if(s < gridSteps - 1) {
                                        const nextPos = (s + 1) * interval;
                                        block += `        if $rtemp_x < ${nextPos.toFixed(6)}\n`;
                                        block += `            $rtemp_x = ${pos.toFixed(6)}\n`;
                                        block += `        else\n`;
                                    } else {
                                        block += `        else\n`;
                                        block += `            $rtemp_x = ${pos.toFixed(6)}\n`;
                                    }
                                }
                                for(let s = 0; s < gridSteps - 1; s++) {
                                    block += `        endif\n`;
                                }
                            }
                            if(isTgtBidir) {
                                if(splitSide === 'left') mappedExpr = '0.5 - $rtemp_x * 0.5';
                                else if(splitSide === 'right') mappedExpr = '0.5 + $rtemp_x * 0.5';
                            }
                            block += `        ${tgtVar} = ${mappedExpr}\n`;
                            if(tgtHasPhysics) {
                                block += `        ${restVar} = ${tgtVar}\n        $vel_${tgtIdx} = 0\n        $linked_active_${tgtIdx} = 1\n`;
                            }
                        }

                        // 碰撞桩开启时，未激活状态下联动块必须保持 no-op（不归零、不暂停自动动画）；
                        // “超出范围归零”仅适用于未开启碰撞桩的普通联动
                        if(overflow === 'reset' && !postEntry) {
                            block += `    else\n`;
                            block += `        ; Out of quad: reset with bounce\n`;
                            if(tgtIsJoystick) {
                                if(tgtHasPhysics) {
                                    block += `        $rest_${tgtIdx}_x = 0\n        $rest_${tgtIdx}_y = 0\n        $linked_active_${tgtIdx} = 0\n`;
                                } else {
                                    block += `        $val_${tgtIdx}_x = 0\n        $val_${tgtIdx}_y = 0\n`;
                                }
                            } else if(tgtHasPhysics) {
                                const resetVal = isTgtBidir ? '0.5' : '0';
                                block += `        ${restVar} = ${resetVal}\n        $linked_active_${tgtIdx} = 0\n`;
                            } else {
                                const resetVal = isTgtBidir ? '0.5' : '0';
                                block += `        ${tgtVar} = ${resetVal}\n`;
                            }
                        }
                        block += `    endif\n`;
                        block += `    endif\n`;
                        
                        linkedSlaveCodeBlocks.push(block);
                        return; // skip default single-block code path
                    }
                    
                    // 非方向模式：原始单块代码路径
                    const inRangeExpr = `${srcVar} >= ${srcMin.toFixed(6)} && ${srcVar} <= ${srcMax.toFixed(6)}`;
                    let block = `    ; Linked: comp${srcIdx}${hasMappingTarget ? ` -> comp${tgtIdx}` : ' -> actions-only'}`;
                    block += ` [${srcMin.toFixed(4)}, ${srcMax.toFixed(4)}]`;
                    if(hasMappingTarget && splitSide !== 'both') block += ` split=${splitSide}`;
                    block += ` overflow=${overflow}\n`;
                    if(srcVarPrelude) block += srcVarPrelude;

                    if(hasRangeActions) {
                        // 边沿触发：进入/离开转换统一在下方两阶段区块中应用（离开优先、进入最后）
                        const actionLabel = `Linked comp${srcIdx}${hasMappingTarget ? ` -> comp${tgtIdx}` : ''} [${srcMin.toFixed(4)}, ${srcMax.toFixed(4)}]`;
                        rangeLeaveTransitions.push({ label: actionLabel, insideExpr: inRangeExpr, stateVar: rangeStateVar, actions: leaveActions });
                        rangeEnterTransitions.push({ label: actionLabel, insideExpr: inRangeExpr, stateVar: rangeStateVar, actions: enterActions });
                    }
                    if(!hasMappingTarget) {
                        linkedSlaveCodeBlocks.push(block);
                        return;
                    }
                    block += `    if ${targetDragGuard}\n`;
                    
                    // 进入区间判断
                    block += `    if ${inRangeExpr}\n`;
                    
                    // 计算比例
                    block += `        $temp = (${srcVar} - ${srcMin.toFixed(6)}) / ${range.toFixed(6)}\n`;
                    block += `        if $temp < 0\n            $temp = 0\n        endif\n`;
                    block += `        if $temp > 1\n            $temp = 1\n        endif\n`;
                    
                    // 格子模式吸附
                    if(isTgtGrid) {
                        const interval = 1.0 / (gridSteps - 1);
                        block += `        ; Grid snap (${gridSteps} steps)\n`;
                        for(let s = 0; s < gridSteps; s++) {
                            let pos = s * interval;
                            if(s < gridSteps - 1) {
                                let nextPos = (s + 1) * interval;
                                block += `        if $temp < ${nextPos.toFixed(6)}\n`;
                                block += `            $temp = ${pos.toFixed(6)}\n`;
                                block += `        else\n`;
                            } else {
                                block += `        else\n`;
                                block += `            $temp = ${pos.toFixed(6)}\n`;
                            }
                        }
                        for(let s = 0; s < gridSteps - 1; s++) {
                            block += `        endif\n`;
                        }
                    }
                    
                    // 双向模式转换后得到最终映射值
                    let mappedExpr;
                    const isSrcBidir = !isSrcJoystick && srcComp.paramMode === '2';
                    const isBidirToDir = isSrcBidir && tgtIsDir && link.joyTargetAxis2 !== null && link.joyTargetAxis2 !== undefined && link.joyTargetAxis2 !== '';
                    if(isBidirToDir) {
                        // 双向滑条→方向摇杆：左半→方向1，右半→方向2
                        const dir2 = parseInt(link.joyTargetAxis2, 10) || 0;
                        const tgtVar2 = `$joy_dir_${tgtIdx}_${dir2}`;
                        block += `    if ${inRangeExpr}\n`;
                        block += `        ; Bidir slider → dir ${link.joyTargetAxis} / ${dir2}\n`;
                        block += `        if ${srcVar} < 0.5\n`;
                        block += `            ${tgtVar} = (0.5 - ${srcVar}) * 2.0\n`;
                        block += `            if ${tgtVar} < 0\n                ${tgtVar} = 0\n            endif\n`;
                        block += `            if ${tgtVar} > 1\n                ${tgtVar} = 1\n            endif\n`;
                        block += `            ${tgtVar2} = 0\n`;
                        block += `        else\n`;
                        block += `            ${tgtVar} = 0\n`;
                        block += `            ${tgtVar2} = (${srcVar} - 0.5) * 2.0\n`;
                        block += `            if ${tgtVar2} < 0\n                ${tgtVar2} = 0\n            endif\n`;
                        block += `            if ${tgtVar2} > 1\n                ${tgtVar2} = 1\n            endif\n`;
                        block += `        endif\n`;
                        block += `    else\n`;
                        block += `        ${tgtVar} = 0\n        ${tgtVar2} = 0\n`;
                        block += `    endif\n`;
                        linkedSlaveCodeBlocks.push(block);
                        return;
                    } else if(tgtIsDir) {
                        mappedExpr = `$temp`; // $joy_dir 值为 0~1，直接使用
                    } else if(isTgtBidir && isSrcJoystick && !isSrcDir && link.joyAxis2 && link.joyAxis2 !== '') {
                        // XY摇杆→双向滑条：双轴分半
                        const rightVar = `$val_${srcIdx}_${link.joyAxis2}`;
                        const leftNormExpr = range > 0 ? `(${srcVar} - ${srcMin.toFixed(6)}) / ${range.toFixed(6)}` : '0';
                        const rightNormExpr = range > 0 ? `(${rightVar} - ${srcMin.toFixed(6)}) / ${range.toFixed(6)}` : '0';
                        block += `        ; Joy → bidir slider: left=${link.joyAxis} right=${link.joyAxis2}\n`;
                        block += `        $tempL = ${leftNormExpr}\n`;
                        block += `        if $tempL < 0\n            $tempL = 0\n        endif\n        if $tempL > 1\n            $tempL = 1\n        endif\n`;
                        block += `        $tempR = ${rightNormExpr}\n`;
                        block += `        if $tempR < 0\n            $tempR = 0\n        endif\n        if $tempR > 1\n            $tempR = 1\n        endif\n`;
                        block += `        ${tgtVar} = 0.5 + ($tempR - $tempL) * 0.5\n`;
                        block += `        if ${tgtVar} < 0\n            ${tgtVar} = 0\n        endif\n        if ${tgtVar} > 1\n            ${tgtVar} = 1\n        endif\n`;
                    } else if(isTgtBidir && isSrcDir && link.joyAxis2 && link.joyAxis2 !== '') {
                        // 方向摇杆→双向滑条：双方向分半
                        const rightVar = `$joy_dir_${srcIdx}_${parseInt(link.joyAxis2,10)||0}`;
                        const leftNormExpr = `(${srcVar} - ${srcMin.toFixed(6)}) / ${range.toFixed(6)}`;
                        const rightNormExpr = `(${rightVar} - ${srcMin.toFixed(6)}) / ${range.toFixed(6)}`;
                        block += `        ; DirJoy → bidir slider: left=dir${link.joyAxis} right=dir${link.joyAxis2}\n`;
                        block += `        $tempL = ${leftNormExpr}\n`;
                        block += `        if $tempL < 0\n            $tempL = 0\n        endif\n        if $tempL > 1\n            $tempL = 1\n        endif\n`;
                        block += `        $tempR = ${rightNormExpr}\n`;
                        block += `        if $tempR < 0\n            $tempR = 0\n        endif\n        if $tempR > 1\n            $tempR = 1\n        endif\n`;
                        block += `        ${tgtVar} = 0.5 + ($tempR - $tempL) * 0.5\n`;
                        block += `        if ${tgtVar} < 0\n            ${tgtVar} = 0\n        endif\n        if ${tgtVar} > 1\n            ${tgtVar} = 1\n        endif\n`;
                    } else if(tgtIsDir) {
                        mappedExpr = `$temp`; // $joy_dir 值为 0~1，直接使用
                    } else if(isTgtBidir) {
                        if(splitSide === 'left') {
                            mappedExpr = `0.5 - $temp * 0.5`;
                        } else if(splitSide === 'right') {
                            mappedExpr = `0.5 + $temp * 0.5`;
                        } else {
                            mappedExpr = `$temp`;
                        }
                    } else if(tgtIsJoystick) {
                        if(splitSide === 'left') {
                            mappedExpr = `$temp - 1`;
                        } else if(splitSide === 'right') {
                            mappedExpr = `$temp`;
                        } else {
                            mappedExpr = `$temp * 2 - 1`;
                        }
                    } else {
                        mappedExpr = `$temp`;
                    }
                    block += `        ${tgtVar} = ${mappedExpr}\n`;
                    // 同步更新弹簧目标（$rest），使物理弹簧不抵抗映射值（方向摇杆不需要）
                    if(!tgtIsDir && tgtHasPhysics) {
                        block += `        ${restVar} = ${tgtVar}\n`;
                        block += `        $vel_${tgtIdx} = 0\n`;
                        // 联动激活：置联动占用闩锁 $linked_active 暂停物理自动驱动，不直接写 $auto（尊重外部绑定，A2 修复）
                        block += `        $linked_active_${tgtIdx} = 1\n`;
                    }
                    
                    // 超出范围处理
                    if(tgtIsDir) {
                        block += `    else\n`;
                        block += `        ; Out of range: reset dir value\n`;
                        block += `        ${tgtVar} = 0\n`;
                    } else if(overflow === 'reset') {
                        const resetVal = tgtIsJoystick ? '0' : (isTgtBidir ? '0.5' : '0');
                        block += `    else\n`;
                        block += `        ; Overflow: reset to ${resetVal}\n`;
                        if(tgtHasPhysics) {
                            // 仅设弹簧目标，不设 val，让弹簧自然产生回弹；同时复位联动闩锁恢复自动驱动（A3 修复）
                            block += `        ${restVar} = ${resetVal}\n`;
                            block += `        $linked_active_${tgtIdx} = 0\n`;
                        } else {
                            // 无物理时直接设值
                            block += `        ${tgtVar} = ${resetVal}\n`;
                        }
                    } else { // 'keep_max'
                        const belowVal = tgtIsJoystick ? '-1' : (isTgtBidir ? '0.5' : '0');
                        block += `    else\n`;
                        block += `        if ${srcVar} < ${srcMin.toFixed(6)}\n`;
                        block += `            ; Below range: hold min\n`;
                        block += `            ${tgtVar} = ${belowVal}\n`;
                        if(tgtHasPhysics) {
                            block += `            ${restVar} = ${belowVal}\n`;
                            block += `            $linked_active_${tgtIdx} = 1\n`;
                        }
                        block += `        else\n`;
                        block += `            ; Above range: hold max\n`;
                        block += `            ${tgtVar} = 1\n`;
                        if(tgtHasPhysics) {
                            block += `            ${restVar} = 1\n`;
                            block += `            $linked_active_${tgtIdx} = 1\n`;
                        }
                        block += `        endif\n`;
                    }
                    
                    block += `    endif\n`;
                    block += `    endif\n`;
                    
                    // 同步 $auto_goal/$auto_tgt 以防与自动动画冲突（方向摇杆不需要）；
                    // 仅在区间激活期间同步：联动未活动时不把目标钉在陈旧映射值上（A7 修复）
                    if(!tgtIsDir && tgtHasPhysics && componentExportUsage[tgtIdx].autoEnabled) {
                        if(tgtIsJoystick) {
                            block += `    if ${targetDragGuard}\n    if ${inRangeExpr}\n    $auto_goal_${tgtIdx}_${tgtAxis} = ${tgtVar}\n    $auto_tgt_${tgtIdx}_${tgtAxis} = ${tgtVar}\n    endif\n    endif\n`;
                        } else {
                            block += `    if ${targetDragGuard}\n    if ${inRangeExpr}\n    $auto_goal_${tgtIdx} = ${tgtVar}\n    $auto_tgt_${tgtIdx} = ${tgtVar}\n    endif\n    endif\n`;
                        }
                    }
                    
                    linkedSlaveCodeBlocks.push(block);
                });
            });
            if(linkedSlaveCodeBlocks.length > 0) {
                t += `\n    ; Linked Slaves Override (before physics for spring bounce)\n`;
                linkedSlaveCodeBlocks.forEach(block => {
                    t += block;
                });
            }

            // ========== Standalone Range Triggers (independent interval triggers without target mapping) ==========
            const rangeTriggerCodeBlocks = [];
            components.forEach((srcComp, srcIdx) => {
                const triggers = Array.isArray(srcComp.rangeTriggers) ? srcComp.rangeTriggers : [];
                triggers.forEach((trigger, triggerIdx) => {
                    if(!trigger) return;
                    const enterActions = Array.isArray(trigger.enterActions)
                        ? trigger.enterActions.filter(a => a && a.var && String(a.var).trim())
                        : [];
                    const leaveActions = Array.isArray(trigger.leaveActions)
                        ? trigger.leaveActions.filter(a => a && a.var && String(a.var).trim())
                        : [];
                    if(enterActions.length === 0 && leaveActions.length === 0) return;

                    const isSrcJoystick = srcComp.type === 'joystick';
                    const rangeStateVar = `$range_state_${srcIdx}_${triggerIdx}`;

                    let inRangeExpr;
                    let rangeLabel;

                    if(isSrcJoystick) {
                        const pts = getRangeTriggerRegionPoints(trigger);
                        const sx = `$val_${srcIdx}_x`;
                        const sy = `$val_${srcIdx}_y`;
                        const crossExprs = [];
                        for(let i = 0; i < 4; i++) {
                            const j = (i + 1) % 4;
                            const ex = pts[j].x - pts[i].x;
                            const ey = pts[j].y - pts[i].y;
                            crossExprs.push(
                                `(${ex.toFixed(6)}) * (${sy} - ${pts[i].y.toFixed(6)}) - (${ey.toFixed(6)}) * (${sx} - ${pts[i].x.toFixed(6)})`
                            );
                        }
                        const qVars = crossExprs.map((_, i) => `$range_quad_${srcIdx}_${triggerIdx}_${i}`);
                        inRangeExpr = `(${qVars.map(v => `${v} >= 0`).join(' && ')}) || (${qVars.map(v => `${v} <= 0`).join(' && ')})`;
                        rangeLabel = `P0(${pts[0].x.toFixed(2)},${pts[0].y.toFixed(2)}) P1(${pts[1].x.toFixed(2)},${pts[1].y.toFixed(2)}) P2(${pts[2].x.toFixed(2)},${pts[2].y.toFixed(2)}) P3(${pts[3].x.toFixed(2)},${pts[3].y.toFixed(2)})`;
                    } else {
                        // 滑块：单轴区间
                        const srcMin = trigger.srcMin ?? 0;
                        const srcMax = trigger.srcMax ?? 0.5;
                        const range = srcMax - srcMin;
                        if(range <= 0) return;
                        inRangeExpr = `$val_${srcIdx} >= ${srcMin.toFixed(6)} && $val_${srcIdx} <= ${srcMax.toFixed(6)}`;
                        rangeLabel = `[${srcMin.toFixed(2)}, ${srcMax.toFixed(2)}]`;
                    }

                    let block = `    ; RangeTrigger: comp${srcIdx} ${rangeLabel}\n`;
                    if(isSrcJoystick) {
                        const pts = getRangeTriggerRegionPoints(trigger);
                        for(let i = 0; i < 4; i++) {
                            const j = (i + 1) % 4;
                            const ex = pts[j].x - pts[i].x;
                            const ey = pts[j].y - pts[i].y;
                            block += `    $range_quad_${srcIdx}_${triggerIdx}_${i} = (${ex.toFixed(6)}) * ($val_${srcIdx}_y - ${pts[i].y.toFixed(6)}) - (${ey.toFixed(6)}) * ($val_${srcIdx}_x - ${pts[i].x.toFixed(6)})\n`;
                        }
                    }
                    // 边沿触发：进入/离开转换统一在下方两阶段区块中应用（离开优先、进入最后）
                    rangeLeaveTransitions.push({ label: `RangeTrigger comp${srcIdx} ${rangeLabel}`, insideExpr: inRangeExpr, stateVar: rangeStateVar, actions: leaveActions });
                    rangeEnterTransitions.push({ label: `RangeTrigger comp${srcIdx} ${rangeLabel}`, insideExpr: inRangeExpr, stateVar: rangeStateVar, actions: enterActions });

                    rangeTriggerCodeBlocks.push(block);
                });
            });
            if(rangeTriggerCodeBlocks.length > 0) {
                t += `\n    ; Range Triggers (standalone interval triggers)\n`;
                rangeTriggerCodeBlocks.forEach(block => {
                    t += block;
                });
            }
            if(rangeLeaveTransitions.length > 0 || rangeEnterTransitions.length > 0) {
                t += `\n    ; Range Actions (edge-triggered: leave transitions first, then enter transitions)\n`;
                t += `    ; 多区间共享动作变量时，先处理所有离开、再处理所有进入，避免后面的块覆盖前面的块\n`;
                rangeLeaveTransitions.forEach(tr => {
                    t += `    ; leave: ${tr.label}\n`;
                    t += `    if ${tr.insideExpr}\n    else\n`;
                    t += `        if ${tr.stateVar} == 1\n`;
                    if(tr.actions.length > 0) {
                        t += buildLinkedSlaveActionINI(tr.actions, '            ');
                    }
                    t += `            ${tr.stateVar} = 0\n`;
                    t += `        endif\n`;
                    t += `    endif\n`;
                });
                rangeEnterTransitions.forEach(tr => {
                    t += `    ; enter: ${tr.label}\n`;
                    t += `    if ${tr.insideExpr}\n`;
                    t += `        if ${tr.stateVar} == 0\n`;
                    if(tr.actions.length > 0) {
                        t += buildLinkedSlaveActionINI(tr.actions, '            ');
                    }
                    t += `            ${tr.stateVar} = 1\n`;
                    t += `        endif\n`;
                    t += `    endif\n`;
                });
            }

            // 物理弹簧（始终运行，但每个组件检测自己的 $is_dragging）
            components.forEach((m,i) => {
                if(m.type==='static' || m.type==='toggle' || m.type==='text' || m.type==='sequence') return;
                const physicsSupported = componentSupportsPhysics(m);
                if(!physicsSupported) return;
                if(!componentExportUsage[i].physicsEnabled) return; // 不勾选物理时不生成任何物理代码
                const physicsSpring = true;
                const autoEnabled = physicsSpring && componentExportUsage[i].autoEnabled;
                const autoSource = getAutoSourceMode(m);
                let autoSamplesX = null;
                let autoSamplesY = null;
                if (autoEnabled && autoSource === 'function') {
                    try {
                        autoSamplesX = buildAutoFunctionSamples(m, 'x');
                        if (m.type === 'joystick') autoSamplesY = buildAutoFunctionSamples(m, 'y');
                    } catch(err) {
                        throw new Error(`组件 ${i + 1} 的自动函数无效: ${err.message}`);
                    }
                }
                t+=`        if $phys_mode_${i} == 1\n`;
                if (m.type === 'joystick') {
                    if(m.physicsProfile === 'breast') {
                        t+=`            $drive_${i}_x = $rest_${i}_x\n`;
                        t+=`            $drive_${i}_y = $rest_${i}_y\n`;
                        t+=`            if $auto_${i} == 1 && $linked_active_${i} == 0\n`;
                        t+=`                if $auto_prev_${i} == 0\n`;
                        if (autoSource === 'chaos') {
                            t+=`                    $chaos_${i}_x = $auto_seed_x_${i}\n`;
                            t+=`                    $chaos_${i}_y = $auto_seed_y_${i}\n`;
                            t+=`                    $counter_${i} = $chaos_rate_${i} - 1\n`;
                        } else {
                            t+=`                    $auto_phase_${i} = 0\n`;
                        }
                        t+=`                    $auto_goal_${i}_x = $val_${i}_x\n                    $auto_goal_${i}_y = $val_${i}_y\n`;
                        t+=`                    $auto_tgt_${i}_x = $val_${i}_x\n                    $auto_tgt_${i}_y = $val_${i}_y\n`;
                        t+=`                    $auto_prev_${i} = 1\n`;
                        t+=`                endif\n`;
                        if (autoSource === 'chaos') {
                            t+=`                $counter_${i} = $counter_${i} + 1\n`;
                            t+=`                if $counter_${i} >= $chaos_rate_${i}\n`;
                            t+=`                    $counter_${i} = 0\n`;
                            t+=`                    $chaos_${i}_x = 3.91 * $chaos_${i}_x * (1 - $chaos_${i}_x)\n`;
                            t+=`                    $chaos_${i}_y = 3.83 * $chaos_${i}_y * (1 - $chaos_${i}_y)\n`;
                            t+=`                    $auto_goal_${i}_x = ($chaos_${i}_x - 0.5) * 2 * $auto_amp_x_${i}\n`;
                            t+=`                    $auto_goal_${i}_y = ($chaos_${i}_y - 0.5) * 2 * $auto_amp_y_${i}\n`;
                            t+=`                endif\n`;
                        } else {
                            t+=`                $auto_phase_${i} = $auto_phase_${i} + (1 / $chaos_rate_${i})\n`;
                            t+=`                if $auto_phase_${i} >= 1\n                    $auto_phase_${i} = $auto_phase_${i} - 1\n                endif\n`;
                            t+= buildAutoSampleAssignment(`$auto_goal_${i}_x`, `$auto_phase_${i}`, autoSamplesX).replace(/^ {16}/gm, '                ');
                            t+= buildAutoSampleAssignment(`$auto_goal_${i}_y`, `$auto_phase_${i}`, autoSamplesY).replace(/^ {16}/gm, '                ');
                        }
                        t+=`                $d1 = $auto_speed_${i}\n`;
                        t+=`                if $d1 < 0.08\n                    $d1 = 0.08\n                endif\n`;
                        t+=`                $d2 = $auto_response_${i}\n`;
                        t+=`                if $d2 < 0.35\n                    $d2 = 0.35\n                endif\n`;
                        t+=`                $temp = ($auto_goal_${i}_x - $auto_tgt_${i}_x) * $d2\n`;
                        t+=`                if $temp > $d1\n $temp = $d1\n endif\n if $temp < -$d1\n $temp = -$d1\n endif\n`;
                        t+=`                $auto_tgt_${i}_x = $auto_tgt_${i}_x + $temp\n`;
                        t+=`                $temp = ($auto_goal_${i}_y - $auto_tgt_${i}_y) * $d2\n`;
                        t+=`                if $temp > $d1\n $temp = $d1\n endif\n if $temp < -$d1\n $temp = -$d1\n endif\n`;
                        t+=`                $auto_tgt_${i}_y = $auto_tgt_${i}_y + $temp\n`;
                        t+=`                $drive_${i}_x = $auto_tgt_${i}_x\n`;
                        t+=`                $drive_${i}_y = $auto_tgt_${i}_y\n`;
                        t+=`            else\n`;
                        t+=`                $auto_prev_${i} = 0\n`;
                        t+=`            endif\n`;
                        t+=`            $force = ($drive_${i}_x - $val_${i}_x) * (($spring_k_${i} * 1.55) + 0.12)\n`;
                        t+=`            $vel_${i}_x = ($vel_${i}_x + $force) * (($spring_d_${i} * 0.72) + 0.16)\n`;
                        t+=`            $val_${i}_x = $val_${i}_x + $vel_${i}_x\n`;
                        t+=`            $force = ($drive_${i}_y - $val_${i}_y) * (($spring_k_${i} * 2.05) + 0.16)\n`;
                        t+=`            $temp = $drive_${i}_y - $val_${i}_y\n`;
                        t+=`            if $temp < 0\n                $temp = -$temp\n            endif\n`;
                        t+=`            $force = $force + $temp * (($spring_k_${i} * 0.30) + 0.03)\n`;
                        t+=`            $temp = $vel_${i}_y\n`;
                        t+=`            if $temp < 0\n                $temp = -$temp\n            endif\n`;
                        t+=`            $force = $force + $temp * (($spring_k_${i} * 0.08) + 0.01)\n`;
                        t+=`            if $gravity_${i} != 0\n                $force = $force - $gravity_${i}\n            endif\n`;
                        t+=`            $vel_${i}_y = ($vel_${i}_y + $force) * (($spring_d_${i} * 0.66) + 0.18)\n`;
                        t+=`            $val_${i}_y = $val_${i}_y + $vel_${i}_y\n`;
                        t+=`            if $val_${i}_x < -1\n $val_${i}_x = -1\n $vel_${i}_x = -$vel_${i}_x * $auto_bounce_${i}\n endif\n if $val_${i}_x > 1\n $val_${i}_x = 1\n $vel_${i}_x = -$vel_${i}_x * $auto_bounce_${i}\n endif\n`;
                        t+=`            if $val_${i}_y < -1\n $val_${i}_y = -1\n $vel_${i}_y = -$vel_${i}_y * $auto_bounce_${i}\n endif\n if $val_${i}_y > 1\n $val_${i}_y = 1\n $vel_${i}_y = -$vel_${i}_y * $auto_bounce_${i}\n endif\n`;
                    } else {
                        t+=`            if $is_dragging == ${i+1} && $drag_action >= 2\n`;
                        t+=`                $vel_${i}_x = 0\n                $vel_${i}_y = 0\n`;
                        if (autoEnabled) {
                            t+=`                $auto_goal_${i}_x = $val_${i}_x\n                $auto_goal_${i}_y = $val_${i}_y\n`;
                            t+=`                $auto_tgt_${i}_x = $val_${i}_x\n                $auto_tgt_${i}_y = $val_${i}_y\n`;
                            if (autoSource === 'chaos') t+=`                $counter_${i} = $chaos_rate_${i} - 1\n`;
                        }
                        t+=`            else\n`;
                        if (autoEnabled) {
                            t+=`                if $auto_${i} == 1 && $linked_active_${i} == 0\n`;
                            t+=`                    if $auto_prev_${i} == 0\n`;
                            if (autoSource === 'chaos') {
                                t+=`                        $chaos_${i}_x = $auto_seed_x_${i}\n`;
                                t+=`                        $chaos_${i}_y = $auto_seed_y_${i}\n`;
                                t+=`                        $counter_${i} = $chaos_rate_${i} - 1\n`;
                            } else {
                                t+=`                        $auto_phase_${i} = 0\n`;
                            }
                            t+=`                        $auto_goal_${i}_x = $val_${i}_x\n                        $auto_goal_${i}_y = $val_${i}_y\n`;
                            t+=`                        $auto_tgt_${i}_x = $val_${i}_x\n                        $auto_tgt_${i}_y = $val_${i}_y\n`;
                            t+=`                        $auto_prev_${i} = 1\n`;
                            t+=`                    endif\n`;
                            if (autoSource === 'chaos') {
                                t+=`                    $d1 = 8 / $chaos_rate_${i}\n`;
                                t+=`                    if $d1 > 1\n                        $d1 = 1\n                    endif\n`;
                                t+=`                    $temp = 3.91 * $chaos_${i}_x * (1 - $chaos_${i}_x)\n`;
                                t+=`                    $chaos_${i}_x = $chaos_${i}_x + ($temp - $chaos_${i}_x) * $d1\n`;
                                t+=`                    $temp = 3.83 * $chaos_${i}_y * (1 - $chaos_${i}_y)\n`;
                                t+=`                    $chaos_${i}_y = $chaos_${i}_y + ($temp - $chaos_${i}_y) * $d1\n`;
                                t+=`                    $auto_goal_${i}_x = ($chaos_${i}_x - 0.5) * 2 * $auto_amp_x_${i}\n`;
                                t+=`                    $auto_goal_${i}_y = ($chaos_${i}_y - 0.5) * 2 * $auto_amp_y_${i}\n`;
                            } else {
                                t+=`                    $auto_phase_${i} = $auto_phase_${i} + (1 / $chaos_rate_${i})\n`;
                                t+=`                    if $auto_phase_${i} >= 1\n                        $auto_phase_${i} = $auto_phase_${i} - 1\n                    endif\n`;
                                t+= buildAutoSampleAssignment(`$auto_goal_${i}_x`, `$auto_phase_${i}`, autoSamplesX).replace(/^ {16}/gm, '                    ');
                                t+= buildAutoSampleAssignment(`$auto_goal_${i}_y`, `$auto_phase_${i}`, autoSamplesY).replace(/^ {16}/gm, '                    ');
                            }
                            t+=`                    $d1 = $auto_speed_${i}\n`;
                            t+=`                    if $d1 < 0.08\n                        $d1 = 0.08\n                    endif\n`;
                            t+=`                    $d2 = $auto_response_${i}\n`;
                            t+=`                    if $d2 < 0.35\n                        $d2 = 0.35\n                    endif\n`;
                            t+=`                    $temp = ($auto_goal_${i}_x - $auto_tgt_${i}_x) * $d2\n`;
                            t+=`                    if $temp > $d1\n $temp = $d1\n endif\n if $temp < -$d1\n $temp = -$d1\n endif\n`;
                            t+=`                    $auto_tgt_${i}_x = $auto_tgt_${i}_x + $temp\n`;
                            t+=`                    $temp = ($auto_goal_${i}_y - $auto_tgt_${i}_y) * $d2\n`;
                            t+=`                    if $temp > $d1\n $temp = $d1\n endif\n if $temp < -$d1\n $temp = -$d1\n endif\n`;
                            t+=`                    $auto_tgt_${i}_y = $auto_tgt_${i}_y + $temp\n`;
                            t+=`                    $d3 = ($spring_k_${i} * 2.2) + $auto_str_${i}\n`;
                            t+=`                    if $d3 < 0.35\n                        $d3 = 0.35\n                    endif\n`;
                            t+=`                    $force = ($auto_tgt_${i}_x - $val_${i}_x) * $d3\n`;
                            t+=`                else\n`;
                            t+=`                    $auto_prev_${i} = 0\n`;
                            t+=`                    $force = ($rest_${i}_x - $val_${i}_x) * $spring_k_${i}\n`;
                            t+=`                endif\n`;
                        } else {
                            t+=`                $auto_prev_${i} = 0\n`;
                            t+=`                $force = ($rest_${i}_x - $val_${i}_x) * $spring_k_${i}\n`;
                        }
                        t+=`                $vel_${i}_x = ($vel_${i}_x + $force) * $spring_d_${i}\n                $val_${i}_x = $val_${i}_x + $vel_${i}_x\n`;
                        if (autoEnabled) {
                            t+=`                if $auto_${i} == 1 && $linked_active_${i} == 0\n`;
                            t+=`                    $force = ($auto_tgt_${i}_y - $val_${i}_y) * $d3\n`;
                            t+=`                else\n`;
                        }
                        t+=`                    $force = ($rest_${i}_y - $val_${i}_y) * $spring_k_${i}\n`;
                        if (autoEnabled) {
                            t+=`                endif\n`;
                        }
                        t+=`                if $gravity_${i} != 0\n                    $force = $force - $gravity_${i}\n                endif\n`;
                        t+=`                $vel_${i}_y = ($vel_${i}_y + $force) * $spring_d_${i}\n                $val_${i}_y = $val_${i}_y + $vel_${i}_y\n`;
                        if ((m.gravity ?? 0) !== 0) {
                            t+=`                if $val_${i}_x < -1\n $val_${i}_x = -1\n $vel_${i}_x = -$vel_${i}_x * $auto_bounce_${i}\n endif\n if $val_${i}_x > 1\n $val_${i}_x = 1\n $vel_${i}_x = -$vel_${i}_x * $auto_bounce_${i}\n endif\n`;
                            t+=`                if $val_${i}_y < -1\n $val_${i}_y = -1\n $vel_${i}_y = -$vel_${i}_y * $auto_bounce_${i}\n endif\n if $val_${i}_y > 1\n $val_${i}_y = 1\n $vel_${i}_y = -$vel_${i}_y * $auto_bounce_${i}\n endif\n`;
                        } else {
                            if (autoEnabled) {
                                t+=`                if $auto_${i} == 1 && $linked_active_${i} == 0\n`;
                                t+=`                if $val_${i}_x < -1\n $val_${i}_x = -1\n $vel_${i}_x = -$vel_${i}_x * $auto_bounce_${i}\n endif\n if $val_${i}_x > 1\n $val_${i}_x = 1\n $vel_${i}_x = -$vel_${i}_x * $auto_bounce_${i}\n endif\n`;
                                t+=`                if $val_${i}_y < -1\n $val_${i}_y = -1\n $vel_${i}_y = -$vel_${i}_y * $auto_bounce_${i}\n endif\n if $val_${i}_y > 1\n $val_${i}_y = 1\n $vel_${i}_y = -$vel_${i}_y * $auto_bounce_${i}\n endif\n`;
                                t+=`                else\n`;
                            }
                            t+=`                if $val_${i}_x < -1\n $val_${i}_x = -1\n $vel_${i}_x = 0\n endif\n if $val_${i}_x > 1\n $val_${i}_x = 1\n $vel_${i}_x = 0\n endif\n`;
                            t+=`                if $val_${i}_y < -1\n $val_${i}_y = -1\n $vel_${i}_y = 0\n endif\n if $val_${i}_y > 1\n $val_${i}_y = 1\n $vel_${i}_y = 0\n endif\n`;
                            if (autoEnabled) {
                                t+=`                endif\n`;
                            }
                        }
                        t+=`            endif\n`;
                    }
                } else {
                    t+=`            if $is_dragging == ${i+1} && $drag_action >= 2\n`;
                    t+=`                $vel_${i} = 0\n`;
                    if (autoEnabled) {
                        t+=`                if $auto_${i} == 1 && $linked_active_${i} == 0\n`;
                        t+=`                    $auto_goal_${i} = $val_${i}\n                    $auto_tgt_${i} = $val_${i}\n`;
                        if (autoSource === 'chaos') t+=`                    $counter_${i} = $chaos_rate_${i} - 1\n`;
                        t+=`                endif\n`;
                    }
                    t+=`            else\n`;
                    if (autoEnabled) {
                        t+=`                if $auto_${i} == 1 && $linked_active_${i} == 0\n`;
                        t+=`                    if $auto_prev_${i} == 0\n`;
                        if (autoSource === 'chaos') {
                            t+=`                        $chaos_${i}_x = $auto_seed_x_${i}\n`;
                            t+=`                        $counter_${i} = $chaos_rate_${i} - 1\n`;
                        } else {
                            t+=`                        $auto_phase_${i} = 0\n`;
                        }
                        t+=`                        $auto_goal_${i} = $val_${i}\n                        $auto_tgt_${i} = $val_${i}\n                        $auto_prev_${i} = 1\n`;
                        t+=`                    endif\n`;
                        if (autoSource === 'chaos') {
                            t+=`                    $d1 = 8 / $chaos_rate_${i}\n`;
                            t+=`                    if $d1 > 1\n                        $d1 = 1\n                    endif\n`;
                            t+=`                    $temp = 3.91 * $chaos_${i}_x * (1 - $chaos_${i}_x)\n`;
                            t+=`                    $chaos_${i}_x = $chaos_${i}_x + ($temp - $chaos_${i}_x) * $d1\n`;
                            if(m.type.includes('slider') && m.paramMode==='2') {
                                t+=`                    $auto_goal_${i} = 0.5 + ($chaos_${i}_x - 0.5) * $auto_amp_x_${i}\n`;
                            } else {
                                t+=`                    $auto_goal_${i} = $chaos_${i}_x * $auto_amp_x_${i}\n`;
                            }
                            t+=`                    if $auto_goal_${i} < 0\n $auto_goal_${i} = 0\n endif\n if $auto_goal_${i} > 1\n $auto_goal_${i} = 1\n endif\n`;
                        } else {
                            t+=`                    $auto_phase_${i} = $auto_phase_${i} + (1 / $chaos_rate_${i})\n`;
                            t+=`                    if $auto_phase_${i} >= 1\n                        $auto_phase_${i} = $auto_phase_${i} - 1\n                    endif\n`;
                            t+= buildAutoSampleAssignment(`$auto_goal_${i}`, `$auto_phase_${i}`, autoSamplesX).replace(/^ {16}/gm, '                    ');
                        }
                        t+=`                    $d1 = $auto_speed_${i}\n`;
                        t+=`                    if $d1 < 0.08\n                        $d1 = 0.08\n                    endif\n`;
                        t+=`                    $d2 = $auto_response_${i}\n`;
                        t+=`                    if $d2 < 0.35\n                        $d2 = 0.35\n                    endif\n`;
                        t+=`                    $temp = ($auto_goal_${i} - $auto_tgt_${i}) * $d2\n`;
                        t+=`                    if $temp > $d1\n $temp = $d1\n endif\n if $temp < -$d1\n $temp = -$d1\n endif\n`;
                        t+=`                    $auto_tgt_${i} = $auto_tgt_${i} + $temp\n`;
                        t+=`                    $d3 = ($spring_k_${i} * 2.2) + $auto_str_${i}\n`;
                        t+=`                    if $d3 < 0.35\n                        $d3 = 0.35\n                    endif\n`;
                        t+=`                    $force = ($auto_tgt_${i} - $val_${i}) * $d3\n`;
                        t+=`                else\n`;
                        t+=`                    $auto_prev_${i} = 0\n`;
                    }
                    t+=`                    $force = ($rest_${i} - $val_${i}) * $spring_k_${i}\n`;
                    if (autoEnabled) {
                        t+=`                endif\n`;
                    }
                    t+=`                if $gravity_${i} != 0\n                    $force = $force - $gravity_${i}\n                endif\n`;
                    t+=`                $vel_${i} = ($vel_${i} + $force) * $spring_d_${i}\n                $val_${i} = $val_${i} + $vel_${i}\n`;
                    if ((m.gravity ?? 0) !== 0) {
                        t+=`                if $val_${i} < 0\n $val_${i} = 0\n $vel_${i} = -$vel_${i} * $auto_bounce_${i}\n endif\n if $val_${i} > 1\n $val_${i} = 1\n $vel_${i} = -$vel_${i} * $auto_bounce_${i}\n endif\n`;
                    } else {
                        t+=`                if $auto_${i} == 1 && $linked_active_${i} == 0\n`;
                        t+=`                if $val_${i} < 0\n $val_${i} = 0\n $vel_${i} = -$vel_${i} * $auto_bounce_${i}\n endif\n if $val_${i} > 1\n $val_${i} = 1\n $vel_${i} = -$vel_${i} * $auto_bounce_${i}\n endif\n`;
                        t+=`                else\n`;
                        t+=`                if $val_${i} < 0\n $val_${i} = 0\n $vel_${i} = 0\n endif\n if $val_${i} > 1\n $val_${i} = 1\n $vel_${i} = 0\n endif\n`;
                        t+=`                endif\n`;
                    }
                    t+=`            endif\n`;
                }
                t+=`        else\n`;
                if (m.type === 'joystick') t+=`            $vel_${i}_x = 0\n $vel_${i}_y = 0\n`;
                else t+=`            $vel_${i} = 0\n`;
                t+=`        endif\n`;
            });

            t+=`    endif\n\n`;

            t+=`\n    ; Clamp joystick center to rounded bounds\n`;
            components.forEach((m,i) => {
                if(m.type !== 'joystick') return;
                t+=`    $sw = $hs_${i}*$zoom_global*$anim_handle_scale_${i}\n    $sh = $hh_${i}*$zoom_global*$anim_handle_scale_${i}\n`;
                t+=`    $d1 = ($abs_w_${i} - $sw) * 0.5\n`;
                t+=`    $d2 = ($abs_h_${i} - $sh) * 0.5\n`;
                t+=`    if $d1 < 0.000001\n        $d1 = 0.000001\n    endif\n`;
                t+=`    if $d2 < 0.000001\n        $d2 = 0.000001\n    endif\n`;
                t+=`    $d3 = $joy_corner_${i} * $zoom_global\n`;
                t+=`    if $d3 < 0\n        $d3 = 0\n    endif\n`;
                t+=`    if $d3 > $d1\n        $d3 = $d1\n    endif\n`;
                t+=`    if $d3 > $d2\n        $d3 = $d2\n    endif\n`;
                t+=`    $cx = $val_${i}_x * $d1\n`;
                t+=`    $cy = -$val_${i}_y * $d2\n`;
                t+=`    if $cx < 0\n        $rx = -$cx\n    else\n        $rx = $cx\n    endif\n`;
                t+=`    if $cy < 0\n        $ry = -$cy\n    else\n        $ry = $cy\n    endif\n`;
                t+=`    $temp = $d1 - $d3\n`;
                t+=`    $force = $d2 - $d3\n`;
                t+=`    if $rx > $temp && $ry > $force\n`;
                t+=`        if $cx < 0\n            $c_rot = -1\n        else\n            $c_rot = 1\n        endif\n`;
                t+=`        if $cy < 0\n            $s_rot = -1\n        else\n            $s_rot = 1\n        endif\n`;
                t+=`        $rx = $rx - $temp\n`;
                t+=`        $ry = $ry - $force\n`;
                t+=`        $w_draw = $rx * $rx + $ry * $ry\n`;
                t+=`        if $w_draw > ($d3 * $d3) && $d3 > 0.000001\n`;
                t+=`            $h_draw = $w_draw\n`;
                t+=`            if $h_draw < 0.000001\n                $h_draw = 0.000001\n            endif\n`;
                t+=`            $h_draw = ($h_draw + ($d3 * $d3) / $h_draw) * 0.5\n`;
                t+=`            $h_draw = ($h_draw + ($d3 * $d3) / $h_draw) * 0.5\n`;
                t+=`            $h_draw = ($h_draw + ($d3 * $d3) / $h_draw) * 0.5\n`;
                t+=`            $h_draw = ($h_draw + ($d3 * $d3) / $h_draw) * 0.5\n`;
                t+=`            $h_draw = ($h_draw + ($d3 * $d3) / $h_draw) * 0.5\n`;
                t+=`            $rx = $rx * $d3 / $h_draw\n`;
                t+=`            $ry = $ry * $d3 / $h_draw\n`;
                t+=`        endif\n`;
                t+=`        $cx = ($temp + $rx) * $c_rot\n`;
                t+=`        $cy = ($force + $ry) * $s_rot\n`;
                t+=`        $val_${i}_x = $cx / $d1\n`;
                t+=`        $val_${i}_y = -($cy / $d2)\n`;
                t+=`    endif\n`;
            });

            components.forEach((m,i) => {
                if(m.type === 'text' || m.type === 'static' || m.type === 'toggle' || m.type === 'sequence' || m.type === 'accum') return;
                t+=`    $sw = $hs_${i}*$zoom_global*$anim_handle_scale_${i}\n    $sh = $hh_${i}*$zoom_global*$anim_handle_scale_${i}\n`;
                let wT=`($abs_w_${i}-$sw)`, hT=`($abs_h_${i}-$sh)`;
                if(m.type==='slider_h') {
                    t+=`    $dx = ($val_${i} * ${wT}) + $sw*0.5 - $abs_w_${i}*0.5 + $anim_handle_dx_${i} * $abs_w_${i}\n    $dy = $anim_handle_dy_${i} * $abs_h_${i}\n`;
                } else if(m.type==='slider_v') {
                    t+=`    $dy = ((1-$val_${i}) * ${hT}) + $sh*0.5 - $abs_h_${i}*0.5 + $anim_handle_dy_${i} * $abs_h_${i}\n    $dx = $anim_handle_dx_${i} * $abs_w_${i}\n`;
                } else {
                    t+=`    $dx = (($val_${i}_x+1)*0.5 * ${wT}) + $sw*0.5 - $abs_w_${i}*0.5 + $anim_handle_dx_${i} * $abs_w_${i}\n`;
                    t+=`    $dy = ((1-$val_${i}_y)*0.5 * ${hT}) + $sh*0.5 - $abs_h_${i}*0.5 + $anim_handle_dy_${i} * $abs_h_${i}\n`;
                }
                t+=`    $rx = $dx * $aspect\n    $ry = $dy\n`;
                t+=`    $r_hdl_${i}_x = $abs_x_${i} + $abs_w_${i}*0.5 + ($rx * $cos_${i} - $ry * $sin_${i}) / $aspect - $sw*0.5\n`;
                t+=`    $r_hdl_${i}_y = $abs_y_${i} + $abs_h_${i}*0.5 + ($rx * $sin_${i} + $ry * $cos_${i}) - $sh*0.5\n`;
            });

            components.forEach((m,i) => {
                if(!isSliderSubdivisionMode(m)) return;
                const sliderCfg = getSliderSubdivisionConfig(m);
                t+=`    ; Slider cumulative subdivisions for component ${i}\n`;
                for(let sideIdx = 0; sideIdx < sliderCfg.sideCount; sideIdx++) {
                    const pressureExpr = m.paramMode === '2'
                        ? (sideIdx === 0 ? `((0.5 - $val_${i}) * 2)` : `(($val_${i} - 0.5) * 2)`)
                        : `$val_${i}`;
                    for(let segIdx = 0; segIdx < sliderCfg.subdivisions; segIdx++) {
                        const flatIdx = getSliderSubdivisionVarIndex(m, sideIdx, segIdx);
                        const lowerBound = (segIdx * sliderCfg.segmentSize).toFixed(6);
                        const segScale = sliderCfg.subdivisions.toFixed(6);
                        t+=`    $slider_seg_${i}_${flatIdx} = ((${pressureExpr}) - ${lowerBound}) * ${segScale}\n`;
                        t+=`    if $slider_seg_${i}_${flatIdx} < 0\n        $slider_seg_${i}_${flatIdx} = 0\n    endif\n`;
                        t+=`    if $slider_seg_${i}_${flatIdx} > 1\n        $slider_seg_${i}_${flatIdx} = 1\n    endif\n`;
                    }
                }
            });

            components.forEach((m,i) => {
                if(m.type !== 'joystick' || m.paramMode !== '4') return;
                const joyCfg = getJoystickConfig(m);
                t+=`    $joy_mix_best_${i} = -1\n`;
                t+=`    $joy_mix_proj_${i} = -1000\n`;
                for(let dirIdx = 0; dirIdx < joyCfg.directionCount; dirIdx++) {
                    for(let segIdx = 0; segIdx < joyCfg.subdivisions; segIdx++) {
                        const flatIdx = getJoystickDirectionVarIndex(m, dirIdx, segIdx);
                        t+=`    $joy_dir_${i}_${flatIdx} = 0\n`;
                    }
                }
                for(let dirIdx = 0; dirIdx < joyCfg.directionCount; dirIdx++) {
                    const nextIdx = (dirIdx + 1) % joyCfg.directionCount;
                    const a = getJoystickDirectionAnchorVector(m, dirIdx);
                    const b = getJoystickDirectionAnchorVector(m, nextIdx);
                    const den = (a.x * b.y) - (a.y * b.x);
                    if(Math.abs(den) <= 0.000001) continue;
                    const crossA = `(${a.x.toFixed(6)} * $val_${i}_y - ${a.y.toFixed(6)} * $val_${i}_x)`;
                    const crossB = `($val_${i}_x * ${b.y.toFixed(6)} - $val_${i}_y * ${b.x.toFixed(6)})`;
                    t+=`    if ${crossA} <= 0.000001 && ${crossB} <= 0.000001\n`;
                    t+=`        $joy_w0_${i} = ($val_${i}_x * ${b.y.toFixed(6)} - $val_${i}_y * ${b.x.toFixed(6)}) / ${den.toFixed(6)}\n`;
                    t+=`        $joy_w1_${i} = (${a.x.toFixed(6)} * $val_${i}_y - ${a.y.toFixed(6)} * $val_${i}_x) / ${den.toFixed(6)}\n`;
                    t+=`        if $joy_w0_${i} < 0\n            $joy_w0_${i} = 0\n        endif\n`;
                    t+=`        if $joy_w1_${i} < 0\n            $joy_w1_${i} = 0\n        endif\n`;
                    for(let segIdx = 0; segIdx < joyCfg.subdivisions; segIdx++) {
                        const flatIdx0 = getJoystickDirectionVarIndex(m, dirIdx, segIdx);
                        const flatIdx1 = getJoystickDirectionVarIndex(m, nextIdx, segIdx);
                        const lowerBound = (segIdx * joyCfg.segmentSize).toFixed(6);
                        const segScale = joyCfg.subdivisions.toFixed(6);
                        t+=`        $joy_dir_${i}_${flatIdx0} = ($joy_w0_${i} - ${lowerBound}) * ${segScale}\n`;
                        t+=`        if $joy_dir_${i}_${flatIdx0} < 0\n            $joy_dir_${i}_${flatIdx0} = 0\n        endif\n`;
                        t+=`        if $joy_dir_${i}_${flatIdx0} > 1\n            $joy_dir_${i}_${flatIdx0} = 1\n        endif\n`;
                        t+=`        $joy_dir_${i}_${flatIdx1} = ($joy_w1_${i} - ${lowerBound}) * ${segScale}\n`;
                        t+=`        if $joy_dir_${i}_${flatIdx1} < 0\n            $joy_dir_${i}_${flatIdx1} = 0\n        endif\n`;
                        t+=`        if $joy_dir_${i}_${flatIdx1} > 1\n            $joy_dir_${i}_${flatIdx1} = 1\n        endif\n`;
                    }
                    t+=`        $joy_mix_best_${i} = 999\n`;
                    t+=`    endif\n`;
                }
                t+=`    if $joy_mix_best_${i} < 0\n`;
                for(let dirIdx = 0; dirIdx < joyCfg.directionCount; dirIdx++) {
                    const anchor = getJoystickDirectionAnchorVector(m, dirIdx);
                    const projExpr = `($val_${i}_x * ${anchor.x.toFixed(6)} + $val_${i}_y * ${anchor.y.toFixed(6)})`;
                    t+=`        $temp = ${projExpr}\n`;
                    t+=`        if $temp > $joy_mix_proj_${i}\n`;
                    t+=`            $joy_mix_proj_${i} = $temp\n`;
                    t+=`            $joy_mix_best_${i} = ${dirIdx}\n`;
                    t+=`        endif\n`;
                }
                t+=`        if $joy_mix_proj_${i} < 0\n            $joy_mix_proj_${i} = 0\n        endif\n`;
                t+=`        if $joy_mix_proj_${i} > 1\n            $joy_mix_proj_${i} = 1\n        endif\n`;
                for(let dirIdx = 0; dirIdx < joyCfg.directionCount; dirIdx++) {
                    for(let segIdx = 0; segIdx < joyCfg.subdivisions; segIdx++) {
                        const flatIdx = getJoystickDirectionVarIndex(m, dirIdx, segIdx);
                        const lowerBound = (segIdx * joyCfg.segmentSize).toFixed(6);
                        const segScale = joyCfg.subdivisions.toFixed(6);
                        t+=`        if $joy_mix_best_${i} == ${dirIdx}\n`;
                        t+=`            $joy_dir_${i}_${flatIdx} = ($joy_mix_proj_${i} - ${lowerBound}) * ${segScale}\n`;
                        t+=`            if $joy_dir_${i}_${flatIdx} < 0\n                $joy_dir_${i}_${flatIdx} = 0\n            endif\n`;
                        t+=`            if $joy_dir_${i}_${flatIdx} > 1\n                $joy_dir_${i}_${flatIdx} = 1\n            endif\n`;
                        t+=`        endif\n`;
                    }
                }
                t+=`    endif\n`;
            });
            t+=`\n    $prev_cursor_x = ${cursorXExpr}\n    $prev_cursor_y = ${cursorYExpr}\n\n`;

            t+=`    ; Sync Bindings (component <-> global, change-detected)\n`;
            // 变动检测双向同步：面板值变化才推送到绑定变量；绑定变量被外部（动画驱动 / 拖拽回读等）
            // 修改时反向跟随进面板值。跟随以槽内第一个绑定变量为准；联动目标与共享镜像组件只推不拉。
            const buildSyncFollowResetCode = (m, i, axis = '') => {
                if(!componentExportUsage[i].physicsEnabled) return '';
                let out = `            $vel_${i}${axis} = 0\n`;
                if(componentExportUsage[i].autoEnabled) {
                    out += `            $auto_goal_${i}${axis} = $val_${i}${axis}\n`;
                    out += `            $auto_tgt_${i}${axis} = $val_${i}${axis}\n`;
                }
                return out;
            };
            bindingComponentOrder.forEach(({ component: m, index: i }) => {
                const writeCond = getBindingWriteCondition(m, i);
                const noFollow = sharedBindingFlags[i] || linkedTargetComponentIds.has(m.id);
                t+=`    if ${componentExportMeta[i].bindingExpr}\n`; 
                if(writeCond) t+=`        if ${writeCond}\n`;
                
                if(m.type==='toggle') {
                    const vars = m.vars.flatMap(s => splitVarStr(s)).filter(v => v !== `$val_${i}`);
                    if(vars.length > 0) {
                        t+=`        if $val_${i} != $vprev_${i}\n`;
                        if(m.switchGroup && m.switchGroup > 0) {
                            t+=`            if $val_${i} > 0\n`;
                            vars.forEach(v => {
                                t+=`                ${v} = $val_${i}\n`;
                            });
                            t+=`            else\n`;
                            vars.forEach(v => {
                                t+=`                ${v} = 0\n`;
                            });
                            t+=`            endif\n`;
                        } else {
                            vars.forEach(v => {
                                t+=`            ${v} = $val_${i}\n`;
                            });
                        }
                        if(!noFollow) {
                            const clampMax = isToggleMultiMode(m) ? `$max_${i}` : `1`;
                            t+=`        else\n`;
                            t+=`            if ${vars[0]} != $val_${i}\n`;
                            t+=`                $val_${i} = ${vars[0]}\n`;
                            t+=`                if $val_${i} < 0\n                    $val_${i} = 0\n                endif\n`;
                            t+=`                if $val_${i} > ${clampMax}\n                    $val_${i} = ${clampMax}\n                endif\n`;
                            t+=`            endif\n`;
                        }
                        t+=`        endif\n`;
                        t+=`        $vprev_${i} = $val_${i}\n`;
                    }
                } else if(m.type==='slider_h' || m.type==='slider_v') {
                    if(isSliderSubdivisionMode(m)) {
                        const sliderCfg = getSliderSubdivisionConfig(m);
                        for(let flatIdx = 0; flatIdx < sliderCfg.totalVars; flatIdx++) {
                            const targetVarStr = (m.vars[flatIdx] && m.vars[flatIdx].trim()) ? m.vars[flatIdx].trim() : `$slider_seg_${i}_${flatIdx}`;
                            const targetVars = splitVarStr(targetVarStr).filter(v => v !== `$slider_seg_${i}_${flatIdx}`);
                            if(targetVars.length === 0) continue;
                            t+=`        if $slider_seg_${i}_${flatIdx} != $segprev_${i}_${flatIdx}\n`;
                            targetVars.forEach(v => {
                                t+=`            ${v} = $slider_seg_${i}_${flatIdx}\n`;
                            });
                            t+=`            $segprev_${i}_${flatIdx} = $slider_seg_${i}_${flatIdx}\n`;
                            t+=`        endif\n`;
                        }
                    } else if(m.paramMode === '2') { 
                        const vLStr = (m.vars[0] && m.vars[0].trim()) ? m.vars[0].trim() : `$val_${i}_L`;
                        const vRStr = (m.vars[1] && m.vars[1].trim()) ? m.vars[1].trim() : `$val_${i}_R`;
                        const vLVars = splitVarStr(vLStr).filter(v => v !== `$val_${i}_L`);
                        const vRVars = splitVarStr(vRStr).filter(v => v !== `$val_${i}_R`);
                        const vLPrimary = vLVars[0] || null;
                        const vRPrimary = vRVars[0] || null;
                        if(vLVars.length > 0 || vRVars.length > 0) {
                            t+=`        if $val_${i} != $vprev_${i}\n`;
                            vLVars.forEach(v => {
                                t+=`            if $val_${i} < 0.5\n  ${v} = (0.5 - $val_${i}) * 2 * $max_${i}\n  else\n  ${v} = 0\n  endif\n`;
                            });
                            vRVars.forEach(v => {
                                t+=`            if $val_${i} > 0.5\n  ${v} = ($val_${i} - 0.5) * 2 * $max_${i}\n  else\n  ${v} = 0\n  endif\n`;
                            });
                            if(!noFollow && vLPrimary && vRPrimary) {
                                t+=`        else\n`;
                                t+=`            if (${vRPrimary} - ${vLPrimary}) != ($val_${i} - 0.5) * 2 * $max_${i}\n`;
                                t+=`                $val_${i} = 0.5 + ((${vRPrimary} - ${vLPrimary}) / (2 * $max_${i}))\n`;
                                t+=`                if $val_${i} < 0\n                    $val_${i} = 0\n                endif\n`;
                                t+=`                if $val_${i} > 1\n                    $val_${i} = 1\n                endif\n`;
                                t+= buildSyncFollowResetCode(m, i);
                                t+=`            endif\n`;
                            }
                            t+=`        endif\n`;
                            t+=`        $vprev_${i} = $val_${i}\n`;
                        }
                    } else if(isSliderGridMode(m)) {
                        const mainVars = getSliderGridMainVars(m);
                        const gridSteps = clampGridStepCount(m.gridSteps, 3);
                        const interval = gridSteps > 1 ? 1.0 / (gridSteps - 1) : 0;
                        if(mainVars.length > 0) {
                            const firstValue = getSliderGridOutputValue(m, 0);
                            const lastValue = getSliderGridOutputValue(m, gridSteps - 1);
                            t+=`        ; 格子模式 (${gridSteps} 档: ${iniNum(firstValue)} -> ${iniNum(lastValue)}, 步长 ${iniNum(getSliderGridValueStep(m))})\n`;
                            t+=`        if $val_${i} != $vprev_${i}\n`;
                            
                            for(let s=0; s<gridSteps; s++) {
                                const outputValue = getSliderGridOutputValue(m, s);
                                const indent = gridSteps === 1 ? '        ' : '            ';
                                const writeMainVars = mainVars.map(v => `${indent}${v} = ${iniNum(outputValue)}\n`).join('');
                                if(gridSteps === 1) {
                                    t+=writeMainVars;
                                    continue;
                                }
                                if(s === gridSteps - 1) {
                                    t+=`        else\n${writeMainVars}`;
                                } else {
                                    let threshold = (s * interval) + interval * 0.5;
                                    if(s===0) t+=`        if $val_${i} < ${threshold.toFixed(6)}\n${writeMainVars}`;
                                    else t+=`        else if $val_${i} < ${threshold.toFixed(6)}\n${writeMainVars}`;
                                }
                            }
                            if(gridSteps > 1) t+=`        endif\n`;
                            t+=`        endif\n`;
                            t+=`        $vprev_${i} = $val_${i}\n`;
                        }
                    } else { 
                        const mainVarStr = (m.vars[0] && m.vars[0].trim()) ? m.vars[0].trim() : `$val_${i}`;
                        const mainVars = splitVarStr(mainVarStr).filter(v => v !== `$val_${i}`);
                        if(mainVars.length > 0) {
                            const useRange = sliderUsesExplicitRange(m);
                            const minVal = useRange ? getSliderRangeMin(m) : 0;
                            const spanVal = useRange ? (getSliderRangeMax(m) - minVal) : 0;
                            const targetExpr = useRange
                                ? `${iniNum(minVal)} + $val_${i} * ${iniNum(spanVal)}`
                                : `$val_${i} * $max_${i}`;
                            t+=`        if $val_${i} != $vprev_${i}\n`;
                            mainVars.forEach(v => {
                                t+=`            ${v} = ${targetExpr}\n`;
                            });
                            if(!noFollow && (!useRange || spanVal !== 0)) {
                                t+=`        else\n`;
                                t+=`            if ${mainVars[0]} != ${targetExpr}\n`;
                                if(useRange) {
                                    t+=`                $val_${i} = (${mainVars[0]} - ${iniNum(minVal)}) / ${iniNum(spanVal)}\n`;
                                } else {
                                    t+=`                $val_${i} = ${mainVars[0]} / $max_${i}\n`;
                                }
                                t+=`                if $val_${i} < 0\n                    $val_${i} = 0\n                endif\n`;
                                t+=`                if $val_${i} > 1\n                    $val_${i} = 1\n                endif\n`;
                                t+= buildSyncFollowResetCode(m, i);
                                t+=`            endif\n`;
                            }
                            t+=`        endif\n`;
                            t+=`        $vprev_${i} = $val_${i}\n`;
                        }
                    }
                } else if(m.type==='joystick') {
                    if(m.paramMode === '4') { 
                        const joyCfg = getJoystickConfig(m);
                        for(let dirIdx = 0; dirIdx < joyCfg.directionCount; dirIdx++) {
                            for(let segIdx = 0; segIdx < joyCfg.subdivisions; segIdx++) {
                                const flatIdx = getJoystickDirectionVarIndex(m, dirIdx, segIdx);
                                const targetVarStr = (m.vars[flatIdx] && m.vars[flatIdx].trim()) ? m.vars[flatIdx].trim() : `$joy_dir_${i}_${flatIdx}`;
                                const targetVars = splitVarStr(targetVarStr).filter(v => v !== `$joy_dir_${i}_${flatIdx}`);
                                if(targetVars.length === 0) continue;
                                const maxVal = (m.maxVals && m.maxVals[flatIdx]) || 1;
                                t+=`        if $joy_dir_${i}_${flatIdx} * ${maxVal} != $dprev_${i}_${flatIdx}\n`;
                                targetVars.forEach(v => {
                                    t+=`            ${v} = $joy_dir_${i}_${flatIdx} * ${maxVal}\n`;
                                });
                                t+=`            $dprev_${i}_${flatIdx} = $joy_dir_${i}_${flatIdx} * ${maxVal}\n`;
                                t+=`        endif\n`;
                            }
                        }
                    } else { 
                        const vXStr = (m.vars[0] && m.vars[0].trim()) ? m.vars[0].trim() : `$val_${i}_x`;
                        const vYStr = (m.vars[1] && m.vars[1].trim()) ? m.vars[1].trim() : `$val_${i}_y`;
                        const vXVars = splitVarStr(vXStr).filter(v => v !== `$val_${i}_x`);
                        const vYVars = splitVarStr(vYStr).filter(v => v !== `$val_${i}_y`);
                        const vXPrimary = vXVars[0] || null;
                        const vYPrimary = vYVars[0] || null;
                        if(vXVars.length > 0) {
                            t+=`        if $val_${i}_x != $vprev_${i}_x\n`;
                            vXVars.forEach(v => { t+=`            ${v} = $val_${i}_x * $max_${i}_x\n`; });
                            if(!noFollow && vXPrimary) {
                                t+=`        else\n`;
                                t+=`            if ${vXPrimary} != $val_${i}_x * $max_${i}_x\n`;
                                t+=`                $val_${i}_x = ${vXPrimary} / $max_${i}_x\n`;
                                t+=`                if $val_${i}_x < -1\n                    $val_${i}_x = -1\n                endif\n`;
                                t+=`                if $val_${i}_x > 1\n                    $val_${i}_x = 1\n                endif\n`;
                                t+= buildSyncFollowResetCode(m, i, '_x');
                                t+=`            endif\n`;
                            }
                            t+=`        endif\n`;
                            t+=`        $vprev_${i}_x = $val_${i}_x\n`;
                        }
                        if(vYVars.length > 0) {
                            t+=`        if $val_${i}_y != $vprev_${i}_y\n`;
                            vYVars.forEach(v => { t+=`            ${v} = $val_${i}_y * $max_${i}_y\n`; });
                            if(!noFollow && vYPrimary) {
                                t+=`        else\n`;
                                t+=`            if ${vYPrimary} != $val_${i}_y * $max_${i}_y\n`;
                                t+=`                $val_${i}_y = ${vYPrimary} / $max_${i}_y\n`;
                                t+=`                if $val_${i}_y < -1\n                    $val_${i}_y = -1\n                endif\n`;
                                t+=`                if $val_${i}_y > 1\n                    $val_${i}_y = 1\n                endif\n`;
                                t+= buildSyncFollowResetCode(m, i, '_y');
                                t+=`            endif\n`;
                            }
                            t+=`        endif\n`;
                            t+=`        $vprev_${i}_y = $val_${i}_y\n`;
                        }
                    }
                }
                
                if(writeCond) t+=`        endif\n`;
                t+=`    endif\n`; 
            });

            t+=`\n    ; Shared Binding Mirror\n`;
            components.forEach((m,i) => {
                if(!sharedBindingFlags[i]) return;
                if(linkedTargetComponentIds.has(m.id)) return;
                const writeCond = getBindingWriteCondition(m, i);
                t+=`    if ${componentExportMeta[i].bindingExpr}\n`;
                if(writeCond) {
                    t+=`        $mirror_gate = 1\n`;
                    t+=`        if ${writeCond}\n            $mirror_gate = 0\n        endif\n`;
                    t+=`        if $mirror_gate == 1\n`;
                }
                if(m.type === 'toggle') {
                    const mainVar = (m.vars[0] && m.vars[0].trim()) ? splitVarStr(m.vars[0].trim())[0] || null : null;
                    if(mainVar) {
                        const clampMax = isToggleMultiMode(m) ? `$max_${i}` : `1`;
                        t+=`            $val_${i} = ${mainVar}\n`;
                        t+=`            if $val_${i} < 0\n                $val_${i} = 0\n            endif\n`;
                        t+=`            if $val_${i} > ${clampMax}\n                $val_${i} = ${clampMax}\n            endif\n`;
                    }
                } else if(m.type === 'slider_h' || m.type === 'slider_v') {
                    if(isSliderSubdivisionMode(m)) {
                        t+=`            ; segmented slider outputs are write-only to avoid a feedback loop\n`;
                    } else if(m.paramMode === '2') {
                        const vLStr = (m.vars[0] && m.vars[0].trim()) ? m.vars[0].trim() : null;
                        const vRStr = (m.vars[1] && m.vars[1].trim()) ? m.vars[1].trim() : null;
                        const vL = vLStr ? splitVarStr(vLStr)[0] || null : null;
                        const vR = vRStr ? splitVarStr(vRStr)[0] || null : null;
                        if(vL && vR) {
                            t+=`            $val_${i} = 0.5 + ((${vR} - ${vL}) / (2 * $max_${i}))\n`;
                            t+=`            if $val_${i} < 0\n                $val_${i} = 0\n            endif\n`;
                            t+=`            if $val_${i} > 1\n                $val_${i} = 1\n            endif\n`;
                        }
                    } else if(isSliderGridMode(m)) {
                        const mainVars = getSliderGridMainVars(m);
                        const mainVar = getSliderGridPrimaryVar(m);
                        const gridSteps = clampGridStepCount(m.gridSteps, 3);
                        if(mainVar) {
                            const gridSpan = (gridSteps - 1) * getSliderGridValueStep(m);
                            if(gridSteps === 1) t+=`            $val_${i} = 0\n`;
                            else t+=`            $val_${i} = (${mainVar} - ${iniNum(getSliderGridValueStart(m))}) / ${iniNum(gridSpan)}\n`;
                            t+=`            if $val_${i} < 0\n                $val_${i} = 0\n            endif\n`;
                            t+=`            if $val_${i} > 1\n                $val_${i} = 1\n            endif\n`;
                            mainVars.forEach((gridVar) => {
                                if(gridVar !== mainVar) t+=`            ${gridVar} = ${mainVar}\n`;
                            });
                        }
                    } else {
                        const mainVarStr = (m.vars[0] && m.vars[0].trim()) ? m.vars[0].trim() : null;
                        const mainVar = mainVarStr ? splitVarStr(mainVarStr)[0] || null : null;
                        if(mainVar) {
                            if(sliderUsesExplicitRange(m)) {
                                const minVal = getSliderRangeMin(m);
                                const spanVal = getSliderRangeSpan(m);
                                t+=`            $val_${i} = (${mainVar} - ${iniNum(minVal)}) / ${iniNum(spanVal)}\n`;
                            } else {
                                t+=`            $val_${i} = ${mainVar} / $max_${i}\n`;
                            }
                            t+=`            if $val_${i} < 0\n                $val_${i} = 0\n            endif\n`;
                            t+=`            if $val_${i} > 1\n                $val_${i} = 1\n            endif\n`;
                        }
                    }
                    if(componentExportUsage[i].physicsEnabled) {
                        t+=`            $vel_${i} = 0\n`;
                        if(componentExportUsage[i].autoEnabled) {
                            t+=`            $auto_goal_${i} = $val_${i}\n`;
                            t+=`            $auto_tgt_${i} = $val_${i}\n`;
                        }
                    }
                } else if(m.type === 'joystick') {
                    if(m.paramMode === '2') {
                        const vXStr = (m.vars[0] && m.vars[0].trim()) ? m.vars[0].trim() : null;
                        const vYStr = (m.vars[1] && m.vars[1].trim()) ? m.vars[1].trim() : null;
                        const vX = vXStr ? splitVarStr(vXStr)[0] || null : null;
                        const vY = vYStr ? splitVarStr(vYStr)[0] || null : null;
                        if(vX) {
                            t+=`            $val_${i}_x = ${vX} / $max_${i}_x\n`;
                            t+=`            if $val_${i}_x < -1\n                $val_${i}_x = -1\n            endif\n`;
                            t+=`            if $val_${i}_x > 1\n                $val_${i}_x = 1\n            endif\n`;
                        }
                        if(vY) {
                            t+=`            $val_${i}_y = ${vY} / $max_${i}_y\n`;
                            t+=`            if $val_${i}_y < -1\n                $val_${i}_y = -1\n            endif\n`;
                            t+=`            if $val_${i}_y > 1\n                $val_${i}_y = 1\n            endif\n`;
                        }
                    } // paramMode '4' 跳过依赖逻辑反馈环（防止输出变量自读累加）
                    if(componentExportUsage[i].physicsEnabled) {
                        t+=`            $vel_${i}_x = 0\n            $vel_${i}_y = 0\n`;
                        if(componentExportUsage[i].autoEnabled) {
                            t+=`            $auto_goal_${i}_x = $val_${i}_x\n            $auto_goal_${i}_y = $val_${i}_y\n`;
                            t+=`            $auto_tgt_${i}_x = $val_${i}_x\n            $auto_tgt_${i}_y = $val_${i}_y\n`;
                        }
                    }
                }
                if(writeCond) t+=`        endif\n`;
                t+=`    endif\n`;
            });

            t+=`\n    ; Recompute After Shared Mirror\n`;
            components.forEach((m,i) => {
                if(m.type === 'text' || m.type === 'static' || m.type === 'toggle' || m.type === 'sequence' || m.type === 'accum') return;
                t+=`    $sw = $hs_${i}*$zoom_global*$anim_handle_scale_${i}\n    $sh = $hh_${i}*$zoom_global*$anim_handle_scale_${i}\n`;
                let wT=`($abs_w_${i}-$sw)`, hT=`($abs_h_${i}-$sh)`;
                if(m.type==='slider_h') {
                    t+=`    $dx = ($val_${i} * ${wT}) + $sw*0.5 - $abs_w_${i}*0.5 + $anim_handle_dx_${i} * $abs_w_${i}\n    $dy = $anim_handle_dy_${i} * $abs_h_${i}\n`;
                } else if(m.type==='slider_v') {
                    t+=`    $dy = ((1-$val_${i}) * ${hT}) + $sh*0.5 - $abs_h_${i}*0.5 + $anim_handle_dy_${i} * $abs_h_${i}\n    $dx = $anim_handle_dx_${i} * $abs_w_${i}\n`;
                } else {
                    t+=`    $dx = (($val_${i}_x+1)*0.5 * ${wT}) + $sw*0.5 - $abs_w_${i}*0.5 + $anim_handle_dx_${i} * $abs_w_${i}\n`;
                    t+=`    $dy = ((1-$val_${i}_y)*0.5 * ${hT}) + $sh*0.5 - $abs_h_${i}*0.5 + $anim_handle_dy_${i} * $abs_h_${i}\n`;
                }
                t+=`    $rx = $dx * $aspect\n    $ry = $dy\n`;
                t+=`    $r_hdl_${i}_x = $abs_x_${i} + $abs_w_${i}*0.5 + ($rx * $cos_${i} - $ry * $sin_${i}) / $aspect - $sw*0.5\n`;
                t+=`    $r_hdl_${i}_y = $abs_y_${i} + $abs_h_${i}*0.5 + ($rx * $sin_${i} + $ry * $cos_${i}) - $sh*0.5\n`;
            });

            // 收集所有直接写入 $joy_dir 的方向摇杆目标，后续方向重建需跳过
            const linkedDirTargets = new Set();
            components.forEach((srcComp, srcIdx) => {
                const slaves = Array.isArray(srcComp.linkedSlaves) ? srcComp.linkedSlaves : [];
                slaves.forEach((link) => {
                    if(!link || !link.enabled || !link.targetId) return;
                    const tgtComp = components.find(c => c.id === link.targetId);
                    if(!tgtComp) return;
                    const tgtIsDir = tgtComp.type === 'joystick' && tgtComp.paramMode === '4';
                    if(!tgtIsDir) return;
                    if(getLinkedSlaveEffectiveRegionMode(link, srcComp, tgtComp) === 'rect') return;
                    const tgtIdx = components.indexOf(tgtComp);
                    if(tgtIdx >= 0) linkedDirTargets.add(tgtIdx);
                });
            });

            components.forEach((m,i) => {
                if(m.type !== 'joystick' || m.paramMode !== '4') return;
                // 跳过那些已在联动逻辑里直接写入 $joy_dir 的方向摇杆目标
                if(linkedDirTargets.has(i)) return;
                const joyCfg = getJoystickConfig(m);
                t+=`    ; direction weights already rebuilt above for joystick ${i}\n`;
            });

            t+=`\n    ; Dependency Logic\n`;
            components.forEach((m,i) => {
                if(m.type==='static' || m.type==='toggle' || m.type==='text' || m.type==='sequence') return;
                if(!componentExportUsage[i].hasDepTargets && !componentExportUsage[i].hasGridDepTargets) return;
                
                let valVar = `$val_${i}`;
                
                if(isSliderSubdivisionMode(m)) {
                    const specs = getComponentDependencyTriggerSpecs(m, i);
                    specs.forEach(spec => {
                        const condExpr = `$slider_seg_${i}_${spec.paramIndex} >= 0.001`;
                        t+=`    if ${componentExportMeta[i].bindingExpr}\n`;
                        t+=`        if ${condExpr}\n`;
                        t+=`            if ${spec.stateVar} == 0\n`;
                        t+=`                ${spec.targetVar} = ${spec.trueValue}\n`;
                        t+=`                ${spec.stateVar} = 1\n`;
                        t+=`            endif\n`;
                        t+=`        else\n`;
                        t+=`            if ${spec.stateVar} != 0\n`;
                        if(spec.useElse) t+=`                ${spec.targetVar} = ${spec.falseValue}\n`;
                        t+=`                ${spec.stateVar} = 0\n`;
                        t+=`            endif\n`;
                        t+=`        endif\n`;
                        t+=`    endif\n`;
                    });
                } else if(m.type.includes('slider') && m.paramMode === '2') {
                    const specs = getComponentDependencyTriggerSpecs(m, i);
                    specs.forEach(spec => {
                        const condExpr = spec.directions.map(dir => dir === 0 ? `$val_${i} < 0.5` : `$val_${i} > 0.5`).join(' || ') || '0';
                        t+=`    if ${componentExportMeta[i].bindingExpr}\n`;
                        t+=`        if ${condExpr}\n`;
                        t+=`            if ${spec.stateVar} == 0\n`;
                        t+=`                ${spec.targetVar} = ${spec.trueValue}\n`;
                        t+=`                ${spec.stateVar} = 1\n`;
                        t+=`            endif\n`;
                        t+=`        else\n`;
                        t+=`            if ${spec.stateVar} != 0\n`;
                        if(spec.useElse) t+=`                ${spec.targetVar} = ${spec.falseValue}\n`;
                        t+=`                ${spec.stateVar} = 0\n`;
                        t+=`            endif\n`;
                        t+=`        endif\n`;
                        t+=`    endif\n`;
                    });
                } else if(m.type.includes('slider') && m.paramMode === '3') {
                    const specs = getComponentDependencyTriggerSpecs(m, i);
                    specs.forEach(spec => {
                        let condExpr;
                        if(spec.gridIndex === 0) condExpr = `$val_${i} < ${spec.thresholdHigh.toFixed(6)}`;
                        else if(spec.gridIndex === spec.gridSteps - 1) condExpr = `$val_${i} >= ${spec.thresholdLow.toFixed(6)}`;
                        else condExpr = `$val_${i} >= ${spec.thresholdLow.toFixed(6)} && $val_${i} < ${spec.thresholdHigh.toFixed(6)}`;
                        t+=`    if ${componentExportMeta[i].bindingExpr}\n`;
                        t+=`        if ${condExpr}\n`;
                        t+=`            if ${spec.stateVar} == 0\n`;
                        t+=`                ${spec.targetVar} = ${spec.trueValue}\n`;
                        t+=`                ${spec.stateVar} = 1\n`;
                        t+=`            endif\n`;
                        t+=`        else\n`;
                        t+=`            if ${spec.stateVar} != 0\n`;
                        if(spec.useElse) t+=`                ${spec.targetVar} = ${spec.falseValue}\n`;
                        t+=`                ${spec.stateVar} = 0\n`;
                        t+=`            endif\n`;
                        t+=`        endif\n`;
                        t+=`    endif\n`;
                    });
                } else {
                    m.depTargets.forEach((targets, vi) => {
                        if(!targets || targets.length === 0) return;
                        
                        targets.forEach(target => {
                            if(!target.var || !target.var.trim()) return;
                            
                            let depTarget = target.var.trim();
                            let inverted = target.invert || false;
                            let useElse = target.else || false;
                            let trueVal = inverted ? 0 : 1;
                            let falseVal = inverted ? 1 : 0;
                            
                            if(m.type === 'joystick') {
                                let valSrc, cond;
                                if(m.paramMode === '4') {
                                    valSrc = `$joy_dir_${i}_${vi}`;
                                    cond = `${valSrc} >= 0.001`;
                                } else {
                                    valSrc = (vi === 0) ? `$val_${i}_x` : `$val_${i}_y`;
                                    cond = `${valSrc} >= 0.001`;
                                }
                                
                                const stateVar = buildDependencyTriggerStateVar(i, vi, targets.indexOf(target));
                                t+=`    if ${componentExportMeta[i].bindingExpr}\n`;
                                t+=`        if ${cond}\n`;
                                t+=`            if ${stateVar} == 0\n`;
                                t+=`                ${depTarget} = ${trueVal}\n`;
                                t+=`                ${stateVar} = 1\n`;
                                t+=`            endif\n`;
                                t+=`        else\n`;
                                t+=`            if ${stateVar} != 0\n`;
                                if(useElse) t+=`                ${depTarget} = ${falseVal}\n`;
                                t+=`                ${stateVar} = 0\n`;
                                t+=`            endif\n`;
                                t+=`        endif\n`;
                                t+=`    endif\n`;
                            } else {
                                const stateVar = buildDependencyTriggerStateVar(i, vi, targets.indexOf(target));
                                t+=`    if ${componentExportMeta[i].bindingExpr}\n`;
                                t+=`        if ${valVar} >= 0.001\n`;
                                t+=`            if ${stateVar} == 0\n`;
                                t+=`                ${depTarget} = ${trueVal}\n`;
                                t+=`                ${stateVar} = 1\n`;
                                t+=`            endif\n`;
                                t+=`        else\n`;
                                t+=`            if ${stateVar} != 0\n`;
                                if(useElse) t+=`                ${depTarget} = ${falseVal}\n`;
                                t+=`                ${stateVar} = 0\n`;
                                t+=`            endif\n`;
                                t+=`        endif\n`;
                                t+=`    endif\n`;
                            }
                        });
                    });
                }
            });

            // ========== 积蓄条：填充比例计算 + 达到阈值触发变量并重置 ==========
            const accumExportList = components.map((m, i) => ({ m, i })).filter(item => item.m && item.m.type === 'accum');
            if(accumExportList.length > 0) {
                t+=`\n    ; Accum Bars (积蓄条)\n`;
                accumExportList.forEach(({ m, i }) => {
                    const accumThreshold = Math.max(0.0001, Number(m.accumThreshold) || 5);
                    t+=`    if $acc_thresh_${i} > 0\n`;
                    t+=`        $acc_fill_${i} = $acc_count_${i} / $acc_thresh_${i}\n`;
                    t+=`        if $acc_fill_${i} < 0\n            $acc_fill_${i} = 0\n        endif\n`;
                    t+=`        if $acc_fill_${i} > 1\n            $acc_fill_${i} = 1\n        endif\n`;
                    t+=`    endif\n`;
                    t+=`    if $acc_count_${i} >= $acc_thresh_${i}\n`;
                    (Array.isArray(m.accumTriggers) ? m.accumTriggers : []).forEach(tr => {
                        if(tr && tr.var && String(tr.var).trim()) {
                            t+=`        ${tr.var} = ${iniNum(Number(tr.value) || 0)}\n`;
                        }
                    });
                    t+=`        $acc_count_${i} = 0\n`;
                    t+=`    endif\n`;
                });
            }
            t+=`\n    ; Render Queue\n`;
            let renderQueue = [];
            const stringifyParam = (value, digits = 3) => typeof value === 'number' ? Number(value).toFixed(digits) : value;
            const exportRoundness = (component, fallback = null, baseExpr = null) => {
                if(fallback != null) {
                    if(typeof fallback === 'string') {
                        return fallback;
                    }
                    const num = Number(fallback);
                    if(Number.isFinite(num)) {
                        if(num >= 0 && baseExpr) {
                            return `-(((${baseExpr})) * ${(num * 0.5).toFixed(6)})`;
                        }
                        return num;
                    }
                    return fallback;
                }
                return -(getComponentCornerRadiusPx(component) / BASE_HEIGHT);
            };
            let baseParams = (i, options = {}) => {
                const cornerRadiusToken = stringifyParam(exportRoundness(components[i], options.roundness, options.roundnessBaseExpr), 6);
                const alphaExpr = options.alphaExpr || `$anim_local_alpha_${i}`;
                const phaseExpr = options.phaseExpr || `$anim_local_sheen_${i}`;
                const rotExpr = options.rotExpr || `$anim_local_rot_${i}`;
                const boostExpr = options.boostExpr || `$anim_fx_boost_${i}`;
                const surfaceFxEnabled = persistentAnim.enabled ? '1' : '0';
                return `x86=$rot_${i}\n y86=$aspect\n z86=${cornerRadiusToken}\n x90=${alphaExpr}\n y90=${phaseExpr}\n z90=${rotExpr}\n w90=${boostExpr}\n x91=${surfaceFxEnabled}\n`;
            };
            let fxParams = (i, mode, color, intensity, width, roundness, options = {}) => {
                const fxColor = Array.isArray(color) ? color : [0.72, 0.92, 1.0];
                const alphaExpr = options.alphaExpr || `$anim_local_alpha_${i}`;
                const phaseExpr = options.phaseExpr || `$anim_local_sheen_${i}`;
                const rotExpr = options.rotExpr || `$anim_local_rot_${i}`;
                const boostExpr = options.boostExpr || `$anim_fx_boost_${i}`;
                const numericMode = Number(mode);
                const timeExpr = options.timeExpr || ((numericMode === 1 || numericMode === 4) ? '0' : '$time');
                return `x88=${Number(mode).toFixed(3)}\n y88=${timeExpr}\n z88=${Number(intensity).toFixed(3)}\n w88=${Number(width).toFixed(3)}\n x89=${Number(fxColor[0]).toFixed(3)}\n y89=${Number(fxColor[1]).toFixed(3)}\n z89=${Number(fxColor[2]).toFixed(3)}\n w89=${stringifyParam(exportRoundness(components[i], roundness, options.roundnessBaseExpr), 6)}\n x90=${alphaExpr}\n y90=${phaseExpr}\n z90=${rotExpr}\n w90=${boostExpr}\n`;
            };
            let drawTexturePass = (i, texExpr, widthExpr, heightExpr, xExpr, yExpr, options = {}) => {
                const passOptions = Object.assign({}, options);
                if(passOptions.resourceKey) {
                    passOptions.alphaExpr = buildResourceAlphaExpr(components[i], passOptions.resourceKey, passOptions.alphaExpr || `$anim_local_alpha_${i}`);
                }
                return `    ps-t100=${texExpr}\n x87=${widthExpr}\n y87=${heightExpr}\n z87=${xExpr}\n w87=${yExpr}\n ${baseParams(i, passOptions)} run=CustomShaderDraw\n`;
            };
            let drawFxPass = (i, widthExpr, heightExpr, xExpr, yExpr, mode, profile, activationExpr = '1', options = {}) => {
                if(!persistentAnim.enabled || !profile || profile.enabled === false || !profile.intensity || profile.intensity <= 0.001) return '';
                const texExpr = options.texExpr || getRes(DEFAULT_ASSET_PATHS.fxWhite);
                const shapeRoundness = options.roundness == null ? null : options.roundness;
                const fxOptions = Object.assign({}, options);
                if(fxOptions.resourceKey) {
                    fxOptions.alphaExpr = buildResourceAlphaExpr(components[i], fxOptions.resourceKey, fxOptions.alphaExpr || `$anim_local_alpha_${i}`);
                }
                const fxBase = baseParams(i, fxOptions) + fxParams(i, mode, profile.color, profile.intensity, profile.width, shapeRoundness, fxOptions);
                return `    if ${activationExpr}\n        ps-t100=${texExpr}\n x87=${widthExpr}\n y87=${heightExpr}\n z87=${xExpr}\n w87=${yExpr}\n ${fxBase} run=CustomShaderFx\n    endif\n`;
            };

            components.forEach((m,i) => {
                let mCmd = "";
                const fxProfile = getComponentFxProfile(m, false);
                const usage = componentExportUsage[i];
                const hasLocalFlow = hasLocalFlowAnimation(usage);
                const hoverFxCond = `$is_dragging == ${i + 1}`;
                const activeFxCond = `$is_dragging == ${i + 1} && $drag_action >= 2`;
                const meta = componentExportMeta[i];
                const visCond = meta.visibilityExpr ? `(${meta.visibilityExpr})` : '';
                // 默认固定（pin）：编组开启后其渲染不受 $help（Home 键）影响，常驻显示。
                // 注意只放宽渲染门控，交互/拖拽命中仍然要求 $help == 1，避免常驻面板在游戏过程中被误拖。
                // 哈希总判定（$active，角色哈希不匹配/人物不绘制时为 0）优先级最高：
                // 固定组只在人物绘制时绕过 Home 常驻显示，人物不绘制时一律不绘制。
                const pinCond = meta.pinExpr && meta.pinExpr !== '0' ? ` || (${meta.pinExpr})` : '';
                let drawCond = `if $active == 1 && ($help == 1${pinCond})\n`;
                let endDraw = `endif\n`;
                if(visCond) {
                    drawCond += `    if ${visCond}\n`;
                    endDraw = `    endif\nendif\n`;
                }
                mCmd += drawCond;
                
                const bgOptions = (m.type === 'toggle' || m.type === 'text')
                    ? { alphaExpr: '1', phaseExpr: '0', boostExpr: '0', resourceKey: 'bg' }
                    : { alphaExpr: '$anim_local_alpha_' + i, resourceKey: 'bg' };
                const bgFxOptions = Object.assign({}, bgOptions, {
                    phaseExpr: '$persistent_phase',
                    timeExpr: '0',
                    boostExpr: '$persistent_boost'
                });
                if(m.paths && m.paths.bg) {
                    mCmd += drawTexturePass(i, getRes(m.paths.bg), `$abs_w_${i}`, `$abs_h_${i}`, `$abs_x_${i}`, `$abs_y_${i}`, bgOptions);
                }
                if(m.type === 'text' && m.textHoverEffect) {
                    mCmd += drawFxPass(i, `$abs_w_${i}`, `$abs_h_${i}`, `$abs_x_${i}`, `$abs_y_${i}`, 2, {
                        enabled: true,
                        color: [0.48, 0.88, 1.0],
                        intensity: 0.72,
                        width: 0.32,
                        roundness: 0.28
                    }, `$text_hover_${i} == 1`, {
                        roundness: 0.28,
                        roundnessBaseExpr: `$abs_h_${i}`,
                        phaseExpr: '0',
                        boostExpr: '0'
                    });
                }
                if(needsFxWhite && persistentAnim.enabled && supportsPersistentSheen(m, usage) && !hasLocalFlow) {
                    mCmd += drawFxPass(i, `$abs_w_${i}`, `$abs_h_${i}`, `$abs_x_${i}`, `$abs_y_${i}`, 1, fxProfile.sheen, '1', bgFxOptions);
                }
                if(needsFxWhite && usage.localAnimProfile.modeCode === 2) {
                    mCmd += drawFxPass(i, `$abs_w_${i}`, `$abs_h_${i}`, `$abs_x_${i}`, `$abs_y_${i}`, 1, fxProfile.sheen, `$anim_local_mode_${i} == 2`, Object.assign({}, bgOptions, {
                        phaseExpr: `$anim_local_sheen_${i}`,
                        timeExpr: '0',
                        boostExpr: `$anim_fx_boost_${i}`
                    }));
                }
                if(needsFxWhite && m.type === 'joystick') {
                    mCmd += drawFxPass(i, `$abs_w_${i}`, `$abs_h_${i}`, `$abs_x_${i}`, `$abs_y_${i}`, 4, {
                        enabled: true,
                        color: [0.52, 0.88, 1.0],
                        intensity: 0.24,
                        width: 0.28,
                        roundness: 0.92
                    }, `$anim_local_mode_${i} == 7`, {
                        phaseExpr: `$anim_local_phase_${i}`,
                        boostExpr: `$anim_fx_boost_${i}`,
                        roundness: 0.92,
                        roundnessBaseExpr: `$abs_h_${i}`,
                        resourceKey: 'bg'
                    });
                }
                
                if(m.type === 'toggle') {
                    if(isToggleMultiMode(m)) {
                        const steps = Math.max(1, m.toggleSteps || DEFAULT_TOGGLE_STEPS);
                        const padX = m.w * 0.04;
                        const padY = m.h * 0.16;
                        const gap = steps > 1 ? Math.min(m.w * 0.012, (m.w - padX * 2) / (steps * 3)) : 0;
                        const usableW = Math.max(m.w - padX * 2 - gap * (steps - 1), m.w * 0.2);
                        const segW = usableW / steps;
                        const segH = Math.max(m.h - padY * 2, m.h * 0.25);
                        const onRes = getRes((m.paths && m.paths.prog_on) ? m.paths.prog_on : TOGGLE_PROGRESS_ASSETS.on);
                        const offRes = getRes((m.paths && m.paths.prog_off) ? m.paths.prog_off : TOGGLE_PROGRESS_ASSETS.off);
                        for(let s = 0; s < steps; s++) {
                            const localX = (-m.w * 0.5) + padX + segW * 0.5 + s * (segW + gap);
                            mCmd += `    $sw = ${segW.toFixed(6)} * $zoom_global * $anim_local_scale_${i}\n    $sh = ${segH.toFixed(6)} * $zoom_global * $anim_local_scale_${i}\n`;
                            mCmd += `    $dx = (${localX.toFixed(6)} * $zoom_global)\n    $dy = 0\n`;
                            mCmd += `    $rx = $dx * $aspect\n    $ry = $dy\n`;
                            mCmd += `    $cx = $abs_x_${i} + $abs_w_${i}*0.5 + ($rx * $cos_${i} - $ry * $sin_${i}) / $aspect\n`;
                            mCmd += `    $cy = $abs_y_${i} + $abs_h_${i}*0.5 + ($rx * $sin_${i} + $ry * $cos_${i})\n`;
                            mCmd += `    if $val_${i} > ${s}\n`;
                            mCmd += drawTexturePass(i, onRes, `$sw`, `$sh`, `$cx - $sw*0.5 + ($anim_state_dx_${i} * $abs_w_${i} * 0.08)`, `$cy - $sh*0.5`, {
                                alphaExpr: `$anim_local_alpha_${i}`,
                                roundness: 0.36,
                                roundnessBaseExpr: `$sh`,
                                boostExpr: `$anim_fx_boost_${i}`,
                                resourceKey: 'prog_on'
                            });
                            mCmd += drawFxPass(i, `$sw`, `$sh`, `$cx - $sw*0.5`, `$cy - $sh*0.5`, 1, fxProfile.sheen, `$anim_local_mode_${i} == 13`, {
                                roundness: 0.36,
                                roundnessBaseExpr: `$sh`,
                                phaseExpr: `$anim_local_phase_${i}`,
                                boostExpr: `$anim_fx_boost_${i}`,
                                resourceKey: 'prog_on'
                            });
                            mCmd += `    else\n`;
                            mCmd += drawTexturePass(i, offRes, `$sw`, `$sh`, `$cx - $sw*0.5`, `$cy - $sh*0.5`, {
                                alphaExpr: '1',
                                roundness: 0.36,
                                roundnessBaseExpr: `$sh`,
                                boostExpr: '0',
                                resourceKey: 'prog_off'
                            });
                            mCmd += drawFxPass(i, `$sw`, `$sh`, `$cx - $sw*0.5`, `$cy - $sh*0.5`, 1, fxProfile.sheen, `$anim_local_mode_${i} == 13`, {
                                roundness: 0.36,
                                roundnessBaseExpr: `$sh`,
                                phaseExpr: `$anim_local_phase_${i}`,
                                boostExpr: `$anim_fx_boost_${i}`,
                                resourceKey: 'prog_off'
                            });
                            mCmd += `    endif\n`;
                        }
                    } else {
                        const offRes = getRes(m.paths.off);
                        const onRes = getRes(m.paths.on);
                        mCmd += `    $w_draw = $abs_w_${i} * $anim_local_scale_${i}\n    $h_draw = $abs_h_${i} * $anim_local_scale_${i}\n`;
                        mCmd += `    $d3 = $abs_y_${i} + ($abs_h_${i} - $h_draw) * 0.5\n`;
                        mCmd += `    if $anim_local_mode_${i} == 8\n`;
                        mCmd += `        if $val_${i} == 0\n`;
                        mCmd += `            $d1 = $abs_x_${i} + ($abs_w_${i} - $w_draw) * 0.5 + ($anim_state_dx_${i} * $abs_w_${i})\n`;
                        mCmd += drawTexturePass(i, offRes, `$w_draw`, `$h_draw`, `$d1`, `$d3`, {
                            alphaExpr: `$anim_local_alpha_${i}`,
                            roundness: 0.44,
                            roundnessBaseExpr: `$h_draw`,
                            boostExpr: `$anim_fx_boost_${i}`,
                            resourceKey: 'off'
                        });
                        mCmd += `            $d2 = $abs_x_${i} + ($abs_w_${i} - $w_draw) * 0.5 - ($anim_state_dx_${i} * $abs_w_${i} * 0.55)\n`;
                        mCmd += drawTexturePass(i, onRes, `$w_draw`, `$h_draw`, `$d2`, `$d3`, {
                            alphaExpr: '0.18',
                            roundness: 0.44,
                            roundnessBaseExpr: `$h_draw`,
                            boostExpr: '0',
                            resourceKey: 'on'
                        });
                        mCmd += `        else\n`;
                        mCmd += `            $d1 = $abs_x_${i} + ($abs_w_${i} - $w_draw) * 0.5 + ($anim_state_dx_${i} * $abs_w_${i})\n`;
                        mCmd += drawTexturePass(i, onRes, `$w_draw`, `$h_draw`, `$d1`, `$d3`, {
                            alphaExpr: `$anim_local_alpha_${i}`,
                            roundness: 0.44,
                            roundnessBaseExpr: `$h_draw`,
                            boostExpr: `$anim_fx_boost_${i}`,
                            resourceKey: 'on'
                        });
                        mCmd += `            $d2 = $abs_x_${i} + ($abs_w_${i} - $w_draw) * 0.5 - ($anim_state_dx_${i} * $abs_w_${i} * 0.55)\n`;
                        mCmd += drawTexturePass(i, offRes, `$w_draw`, `$h_draw`, `$d2`, `$d3`, {
                            alphaExpr: '0.18',
                            roundness: 0.44,
                            roundnessBaseExpr: `$h_draw`,
                            boostExpr: '0',
                            resourceKey: 'off'
                        });
                        mCmd += `        endif\n`;
                        mCmd += `    else\n`;
                        mCmd += `        if $val_${i} == 0\n`;
                        mCmd += drawTexturePass(i, offRes, `$w_draw`, `$h_draw`, `$abs_x_${i} + ($abs_w_${i} - $w_draw) * 0.5`, `$d3`, {
                            alphaExpr: `$anim_local_alpha_${i}`,
                            roundness: 0.44,
                            roundnessBaseExpr: `$h_draw`,
                            boostExpr: `$anim_fx_boost_${i}`,
                            resourceKey: 'off'
                        });
                        mCmd += `        else\n`;
                        mCmd += drawTexturePass(i, onRes, `$w_draw`, `$h_draw`, `$abs_x_${i} + ($abs_w_${i} - $w_draw) * 0.5`, `$d3`, {
                            alphaExpr: `$anim_local_alpha_${i}`,
                            roundness: 0.44,
                            roundnessBaseExpr: `$h_draw`,
                            boostExpr: `$anim_fx_boost_${i}`,
                            resourceKey: 'on'
                        });
                        mCmd += `        endif\n`;
                        mCmd += `    endif\n`;
                        mCmd += drawFxPass(i, `$w_draw`, `$h_draw`, `$abs_x_${i} + ($abs_w_${i} - $w_draw) * 0.5`, `$d3`, 1, fxProfile.sheen, `$anim_local_mode_${i} == 13 && $val_${i} == 0`, {
                            roundness: 0.44,
                            roundnessBaseExpr: `$h_draw`,
                            phaseExpr: `$anim_local_phase_${i}`,
                            boostExpr: `$anim_fx_boost_${i}`,
                            resourceKey: 'off'
                        });
                        mCmd += drawFxPass(i, `$w_draw`, `$h_draw`, `$abs_x_${i} + ($abs_w_${i} - $w_draw) * 0.5`, `$d3`, 1, fxProfile.sheen, `$anim_local_mode_${i} == 13 && $val_${i} != 0`, {
                            roundness: 0.44,
                            roundnessBaseExpr: `$h_draw`,
                            phaseExpr: `$anim_local_phase_${i}`,
                            boostExpr: `$anim_fx_boost_${i}`,
                            resourceKey: 'on'
                        });
                    }
               }
                else if(m.type === 'accum') {
                    const isV = m.accumDirection === 'v';
                    const trackRoundness = 0.45;
                    const trackRes = m.paths && m.paths.bar_r ? getRes(m.paths.bar_r) : '';
                    const fillRes = m.paths && m.paths.bar_l ? getRes(m.paths.bar_l) : '';
                    mCmd += `    $tt = $tt_${i}*$zoom_global\n`;
                    if(trackRes) {
                        if(isV) {
                            mCmd += drawTexturePass(i, trackRes, `$tt`, `$abs_h_${i}`, `$abs_x_${i} + $abs_w_${i}*0.5 - $tt*0.5`, `$abs_y_${i}`, { roundness: trackRoundness, roundnessBaseExpr: `$tt`, resourceKey: 'bar_r' });
                        } else {
                            mCmd += drawTexturePass(i, trackRes, `$abs_w_${i}`, `$tt`, `$abs_x_${i}`, `$abs_y_${i} + $abs_h_${i}*0.5 - $tt*0.5`, { roundness: trackRoundness, roundnessBaseExpr: `$tt`, resourceKey: 'bar_r' });
                        }
                    }
                    if(fillRes) {
                        if(isV) {
                            mCmd += `    $h_draw = $acc_fill_${i} * $abs_h_${i}\n`;
                            mCmd += drawTexturePass(i, fillRes, `$tt`, `$h_draw`, `$abs_x_${i} + $abs_w_${i}*0.5 - $tt*0.5`, `$abs_y_${i} + $abs_h_${i} - $h_draw`, {
                                alphaExpr: `$anim_fill_alpha_${i}`,
                                roundness: trackRoundness,
                                roundnessBaseExpr: `$tt`,
                                boostExpr: `$anim_fx_boost_${i}`,
                                resourceKey: 'bar_l'
                            });
                        } else {
                            mCmd += `    $w_draw = $acc_fill_${i} * $abs_w_${i}\n`;
                            mCmd += drawTexturePass(i, fillRes, `$w_draw`, `$tt`, `$abs_x_${i}`, `$abs_y_${i} + $abs_h_${i}*0.5 - $tt*0.5`, {
                                alphaExpr: `$anim_fill_alpha_${i}`,
                                roundness: trackRoundness,
                                roundnessBaseExpr: `$tt`,
                                boostExpr: `$anim_fx_boost_${i}`,
                                resourceKey: 'bar_l'
                            });
                        }
                    }
                }
               else if(m.type === 'static' && m.paths.img) {
                    mCmd += drawTexturePass(i, getRes(m.paths.img), `$abs_w_${i}`, `$abs_h_${i}`, `$abs_x_${i}`, `$abs_y_${i}`, { resourceKey: 'img' });
                } else if(m.type === 'sequence' && m.seqVar) {
                    m.frames.forEach(f => {
                        if(f.path) {
                            mCmd += `    if ${m.seqVar} == ${f.val}\n`;
                            mCmd += drawTexturePass(i, getRes(f.path), `$abs_w_${i}`, `$abs_h_${i}`, `$abs_x_${i}`, `$abs_y_${i}`);
                            mCmd += `    endif\n`;
                        }
                    });
                } else if(m.type === 'text' && m.textContent) {
                    mCmd += `    ; Text Block ${i} \n`;
                    const textArea = getWorkAreaPixelSize();
                    const textLayout = buildRenderedTextLayout(m, { valueReplacement: '100' });
                    if(textLayout.layout.length > 0) {
                        const glyphCount = Math.max(textLayout.layout.length, 1);
                        const hasDynamicDigits = textLayout.layout.some(slot => slot.isDynamic);
                        if(hasDynamicDigits && m.valVar) {
                            mCmd += `    $val_int = ${m.valVar}\n`;
                            mCmd += `    if $val_int < 0\n        $val_int = 0\n    endif\n`;
                            mCmd += `    if $val_int > 999\n        $val_int = 999\n    endif\n`;
                            const appendDigitResolve = (digitVar, remainderExpr, placeValue) => {
                                mCmd += `    ${digitVar} = 0\n`;
                                for(let digit = 9; digit >= 1; digit--) {
                                    const keyword = digit === 9 ? 'if' : 'else if';
                                    mCmd += `    ${keyword} ${remainderExpr} >= ${digit * placeValue}\n        ${digitVar} = ${digit}\n`;
                                }
                                mCmd += `    endif\n`;
                            };
                            appendDigitResolve('$d0', '$val_int', 100);
                            appendDigitResolve('$d1', '($val_int - $d0 * 100)', 10);
                            appendDigitResolve('$d2', '($val_int - $d0 * 100 - $d1 * 10)', 1);
                        }
                        textLayout.layout.forEach((slot, slotIndex) => {
                            if(!slot.char.trim()) return;
                            const locX = slot.xUnit + (slot.widthPx / textArea.width) * 0.5 - m.w * 0.5;
                            const locY = slot.yUnit + (slot.heightPx / textArea.height) * 0.5 - m.h * 0.5;
                            const waveFactor = Math.sin(((slotIndex / glyphCount) * Math.PI * 2) + slot.index * 0.08);
                            const waveExpr = Math.abs(waveFactor) > 0.0001
                                ? (waveFactor >= 0
                                    ? ` + ($anim_text_wave_${i} * ${waveFactor.toFixed(6)} * $abs_h_${i})`
                                    : ` - ($anim_text_wave_${i} * ${Math.abs(waveFactor).toFixed(6)} * $abs_h_${i})`)
                                : '';
                            const glowProfile = {
                                enabled: true,
                                color: colorToRgbArray(slot.color, [1, 1, 1]),
                                intensity: 0.22,
                                width: 0.18,
                                roundness: 0.08
                            };
                            const hoverScaleExpr = m.textHoverEffect ? `(1 + $text_hover_${i} * 0.08)` : '1';
                            mCmd += `    $sh = ${(slot.heightPx / textArea.height).toFixed(6)} * $zoom_global * ${hoverScaleExpr}\n`;
                            mCmd += `    $dx = (${locX.toFixed(6)} * $zoom_global * ${hoverScaleExpr})\n    $dy = (${locY.toFixed(6)} * $zoom_global * ${hoverScaleExpr}${waveExpr})\n`;
                            mCmd += `    $rx = $dx * $aspect\n    $ry = $dy\n`;
                            mCmd += `    $cx = $abs_x_${i} + $abs_w_${i}*0.5 + ($rx * $cos_${i} - $ry * $sin_${i}) / $aspect\n`;
                            mCmd += `    $cy = $abs_y_${i} + $abs_h_${i}*0.5 + ($rx * $sin_${i} + $ry * $cos_${i})\n`;
                            const appendGlyphPasses = (char, widthPx) => {
                                const resKey = getTextGlyphResourceKey(m, char, slot.color);
                                mCmd += `    $sw = ${(widthPx / textArea.width).toFixed(6)} * $zoom_global * ${hoverScaleExpr}\n`;
                                mCmd += drawTexturePass(i, `Resource_Char_${resKey}`, `$sw`, `$sh`, `$cx - $sw*0.5`, `$cy - $sh*0.5`, {
                                    roundness: 0.08,
                                    roundnessBaseExpr: `$sh`,
                                    phaseExpr: `$anim_local_phase_${i}`,
                                    boostExpr: `$anim_fx_boost_${i}`
                                });
                                mCmd += drawFxPass(i, `$sw`, `$sh`, `$cx - $sw*0.5`, `$cy - $sh*0.5`, 5, glowProfile, `$anim_local_mode_${i} == 11`, {
                                    texExpr: `Resource_Char_${resKey}`,
                                    roundness: 0.08,
                                    roundnessBaseExpr: `$sh`,
                                    phaseExpr: `$anim_local_phase_${i}`,
                                    boostExpr: `$anim_fx_boost_${i}`
                                });
                            };
                            if(slot.isDynamic) {
                                const digitVar = ['$d0', '$d1', '$d2'][slot.dynamicDigitIndex] || '$d2';
                                for(let digit = 0; digit <= 9; digit++) {
                                    mCmd += digit === 0 ? `    if ${digitVar} == 0\n` : `    else if ${digitVar} == ${digit}\n`;
                                    const widthPx = Math.max(2, Math.round(textLayout.charPx * getTextGlyphAdvanceRatio(m, String(digit))));
                                    appendGlyphPasses(String(digit), widthPx);
                                }
                                mCmd += `    endif\n`;
                            } else {
                                appendGlyphPasses(slot.char, slot.widthPx);
                            }
                        });
                    }
                } else if (m.type === 'slider_h' || m.type === 'slider_v') {
                    const isVertical = m.type === 'slider_v';
                    const trackRoundness = 0.45;
                    const trackRes = m.paths && m.paths.bar_r ? getRes(m.paths.bar_r) : '';
                    const fillRes = m.paths && m.paths.bar_l ? getRes(m.paths.bar_l) : '';
                    const handleRes = m.paths && m.paths.handle ? getRes(m.paths.handle) : '';
                    mCmd += `    $sw = $hs_${i}*$zoom_global*$anim_handle_scale_${i}\n    $sh = $hh_${i}*$zoom_global*$anim_handle_scale_${i}\n    $tt = $tt_${i}*$zoom_global\n`;
                    if(trackRes) {
                        if(isVertical) {
                            mCmd += drawTexturePass(i, trackRes, `$tt`, `$abs_h_${i}`, `$abs_x_${i} + $abs_w_${i}*0.5 - $tt*0.5`, `$abs_y_${i}`, { roundness: trackRoundness, roundnessBaseExpr: `$tt`, resourceKey: 'bar_r' });
                            mCmd += drawFxPass(i, `$tt`, `$abs_h_${i}`, `$abs_x_${i} + $abs_w_${i}*0.5 - $tt*0.5`, `$abs_y_${i}`, 1, fxProfile.sheen, `$anim_local_mode_${i} == 13`, {
                                roundness: trackRoundness,
                                roundnessBaseExpr: `$tt`,
                                phaseExpr: `$anim_local_phase_${i}`,
                                boostExpr: `$anim_fx_boost_${i}`,
                                resourceKey: 'bar_r'
                            });
                        } else {
                            mCmd += drawTexturePass(i, trackRes, `$abs_w_${i}`, `$tt`, `$abs_x_${i}`, `$abs_y_${i} + $abs_h_${i}*0.5 - $tt*0.5`, { roundness: trackRoundness, roundnessBaseExpr: `$tt`, resourceKey: 'bar_r' });
                            mCmd += drawFxPass(i, `$abs_w_${i}`, `$tt`, `$abs_x_${i}`, `$abs_y_${i} + $abs_h_${i}*0.5 - $tt*0.5`, 1, fxProfile.sheen, `$anim_local_mode_${i} == 13`, {
                                roundness: trackRoundness,
                                roundnessBaseExpr: `$tt`,
                                phaseExpr: `$anim_local_phase_${i}`,
                                boostExpr: `$anim_fx_boost_${i}`,
                                resourceKey: 'bar_r'
                            });
                        }
                    }
                    if(m.paramMode === '3') {
                        const tickRes = getRes(DEFAULT_ASSET_PATHS.gridTick);
                        const gridSteps = Math.max(2, m.gridSteps || 3);
                        const tickW = isVertical ? Math.max(m.w * 0.18, 0.010) : Math.max(m.w * 0.0045, 0.0014);
                        const tickH = isVertical ? Math.max(m.h * 0.0045, 0.0014) : Math.max(m.h * 0.18, 0.010);
                        for(let s = 0; s < gridSteps; s++) {
                            const ratio = (s / Math.max(gridSteps - 1, 1)).toFixed(6);
                            if(isVertical) {
                                mCmd += drawTexturePass(i, tickRes, `${tickW.toFixed(6)} * $zoom_global`, `${tickH.toFixed(6)} * $zoom_global`, `$abs_x_${i} + $abs_w_${i}*0.5 - (${tickW.toFixed(6)} * $zoom_global)*0.5`, `$abs_y_${i} + (1 - ${ratio}) * $abs_h_${i} - (${tickH.toFixed(6)} * $zoom_global)*0.5`, {
                                    alphaExpr: '0.52',
                                    roundness: 0.14,
                                    roundnessBaseExpr: `${Math.min(tickW, tickH).toFixed(6)} * $zoom_global`,
                                    boostExpr: '0'
                                });
                            } else {
                                mCmd += drawTexturePass(i, tickRes, `${tickW.toFixed(6)} * $zoom_global`, `${tickH.toFixed(6)} * $zoom_global`, `$abs_x_${i} + ${ratio} * $abs_w_${i} - (${tickW.toFixed(6)} * $zoom_global)*0.5`, `$abs_y_${i} + $abs_h_${i}*0.5 - (${tickH.toFixed(6)} * $zoom_global)*0.5`, {
                                    alphaExpr: '0.52',
                                    roundness: 0.14,
                                    roundnessBaseExpr: `${Math.min(tickW, tickH).toFixed(6)} * $zoom_global`,
                                    boostExpr: '0'
                                });
                            }
                        }
                    }
                    if(fillRes) {
                        if(m.paramMode === '2') {
                            if(isVertical) {
                                mCmd += `    if $val_${i} < 0.5\n`;
                                mCmd += `        $h_draw = (0.5 - $val_${i}) * $abs_h_${i}\n`;
                                mCmd += drawTexturePass(i, fillRes, `$tt`, `$h_draw`, `$abs_x_${i} + $abs_w_${i}*0.5 - $tt*0.5`, `$abs_y_${i} + $abs_h_${i}*0.5`, {
                                    alphaExpr: `$anim_fill_alpha_${i}`,
                                    roundness: trackRoundness,
                                    roundnessBaseExpr: `$tt`,
                                    boostExpr: `$anim_fx_boost_${i}`,
                                    resourceKey: 'bar_l'
                                });
                                mCmd += `    else if $val_${i} > 0.5\n`;
                                mCmd += `        $h_draw = ($val_${i} - 0.5) * $abs_h_${i}\n`;
                                mCmd += drawTexturePass(i, fillRes, `$tt`, `$h_draw`, `$abs_x_${i} + $abs_w_${i}*0.5 - $tt*0.5`, `$abs_y_${i} + $abs_h_${i}*0.5 - $h_draw`, {
                                    alphaExpr: `$anim_fill_alpha_${i}`,
                                    roundness: trackRoundness,
                                    roundnessBaseExpr: `$tt`,
                                    boostExpr: `$anim_fx_boost_${i}`,
                                    resourceKey: 'bar_l'
                                });
                                mCmd += `    endif\n`;
                            } else {
                                mCmd += `    if $val_${i} < 0.5\n`;
                                mCmd += `        $w_draw = (0.5 - $val_${i}) * $abs_w_${i}\n`;
                                mCmd += drawTexturePass(i, fillRes, `$w_draw`, `$tt`, `$abs_x_${i} + $abs_w_${i}*0.5 - $w_draw`, `$abs_y_${i} + $abs_h_${i}*0.5 - $tt*0.5`, {
                                    alphaExpr: `$anim_fill_alpha_${i}`,
                                    roundness: trackRoundness,
                                    roundnessBaseExpr: `$tt`,
                                    boostExpr: `$anim_fx_boost_${i}`,
                                    resourceKey: 'bar_l'
                                });
                                mCmd += `    else if $val_${i} > 0.5\n`;
                                mCmd += `        $w_draw = ($val_${i} - 0.5) * $abs_w_${i}\n`;
                                mCmd += drawTexturePass(i, fillRes, `$w_draw`, `$tt`, `$abs_x_${i} + $abs_w_${i}*0.5`, `$abs_y_${i} + $abs_h_${i}*0.5 - $tt*0.5`, {
                                    alphaExpr: `$anim_fill_alpha_${i}`,
                                    roundness: trackRoundness,
                                    roundnessBaseExpr: `$tt`,
                                    boostExpr: `$anim_fx_boost_${i}`,
                                    resourceKey: 'bar_l'
                                });
                                mCmd += `    endif\n`;
                            }
                        } else if(isVertical) {
                            mCmd += `    $h_draw = $val_${i} * $abs_h_${i}\n`;
                            mCmd += drawTexturePass(i, fillRes, `$tt`, `$h_draw`, `$abs_x_${i} + $abs_w_${i}*0.5 - $tt*0.5`, `$abs_y_${i} + $abs_h_${i} - $h_draw`, {
                                alphaExpr: `$anim_fill_alpha_${i}`,
                                roundness: trackRoundness,
                                roundnessBaseExpr: `$tt`,
                                boostExpr: `$anim_fx_boost_${i}`,
                                resourceKey: 'bar_l'
                            });
                        } else {
                            mCmd += `    $w_draw = $val_${i} * $abs_w_${i}\n`;
                            mCmd += drawTexturePass(i, fillRes, `$w_draw`, `$tt`, `$abs_x_${i}`, `$abs_y_${i} + $abs_h_${i}*0.5 - $tt*0.5`, {
                                alphaExpr: `$anim_fill_alpha_${i}`,
                                roundness: trackRoundness,
                                roundnessBaseExpr: `$tt`,
                                boostExpr: `$anim_fx_boost_${i}`,
                                resourceKey: 'bar_l'
                            });
                        }
                    }
                    if(handleRes) {
                        mCmd += drawTexturePass(i, handleRes, `$sw`, `$sh`, `$r_hdl_${i}_x`, `$r_hdl_${i}_y`, {
                            alphaExpr: `$anim_handle_alpha_${i}`,
                            roundness: 0.96,
                            roundnessBaseExpr: `$sh`,
                            boostExpr: `$anim_fx_boost_${i}`,
                            resourceKey: 'handle'
                        });
                        mCmd += drawFxPass(i, `$sw`, `$sh`, `$r_hdl_${i}_x`, `$r_hdl_${i}_y`, 1, fxProfile.sheen, `$anim_local_mode_${i} == 13`, {
                            alphaExpr: `$anim_handle_alpha_${i}`,
                            roundness: 0.96,
                            roundnessBaseExpr: `$sh`,
                            phaseExpr: `$anim_local_phase_${i}`,
                            boostExpr: `$anim_fx_boost_${i}`,
                            resourceKey: 'handle'
                        });
                    }
                } else if (m.type === 'joystick') {
                    const handleRes = m.paths && m.paths.handle ? getRes(m.paths.handle) : '';
                    const postRes = getRes((m.paths && (m.paths.post || m.paths.post_marker)) ? (m.paths.post || m.paths.post_marker) : DEFAULT_ASSET_PATHS.postMarker);
                    const collisionPosts = getJoystickCollisionPosts(m);
                    const joyHandleMetrics = getComponentHandleMetrics(m);
                    const joyTravelUnit = Math.min(
                        Math.max(0, m.w - joyHandleMetrics.width / Math.max(getWorkAreaPixelSize().width, 1)),
                        Math.max(0, m.h - joyHandleMetrics.height / Math.max(getWorkAreaPixelSize().height, 1))
                    );
                    mCmd += `    $sw = $hs_${i}*$zoom_global*$anim_handle_scale_${i}\n    $sh = $hh_${i}*$zoom_global*$anim_handle_scale_${i}\n`;
                    collisionPosts.forEach((post) => {
                        const postRadius = clamp(Number(post.radius) || 0.25, 0.02, 0.5);
                        const workAreaSize = getWorkAreaPixelSize();
                        const postSizePx = Math.max(4, postRadius * Math.min(Math.max(0, m.w * workAreaSize.width - joyHandleMetrics.width), Math.max(0, m.h * workAreaSize.height - joyHandleMetrics.height)) * 0.56);
                        const postWidth = postSizePx / Math.max(workAreaSize.width, 1);
                        const postHeight = postSizePx / Math.max(workAreaSize.height, 1);
                        const postCenterXExpr = `$abs_x_${i} + $abs_w_${i}*0.5 + ((${post.posX.toFixed(6)}) * (($abs_w_${i} - $sw) * 0.5))`;
                        const postCenterYExpr = `$abs_y_${i} + $abs_h_${i}*0.5 - ((${post.posY.toFixed(6)}) * (($abs_h_${i} - $sh) * 0.5))`;
                        mCmd += drawTexturePass(i, postRes, `${postWidth.toFixed(6)} * $zoom_global`, `${postHeight.toFixed(6)} * $zoom_global`, `${postCenterXExpr} - (${postWidth.toFixed(6)} * $zoom_global) * 0.5`, `${postCenterYExpr} - (${postHeight.toFixed(6)} * $zoom_global) * 0.5`, {
                            alphaExpr: '0.5',
                            roundness: 0.98,
                            roundnessBaseExpr: `${Math.min(postWidth, postHeight).toFixed(6)} * $zoom_global`,
                            boostExpr: '0',
                            resourceKey: 'post'
                        });
                    });
                    if(handleRes) {
                        mCmd += drawTexturePass(i, handleRes, `$sw`, `$sh`, `$r_hdl_${i}_x`, `$r_hdl_${i}_y`, {
                            alphaExpr: `$anim_handle_alpha_${i}`,
                            roundness: 0.96,
                            roundnessBaseExpr: `$sh`,
                            boostExpr: `$anim_fx_boost_${i}`,
                            resourceKey: 'handle'
                        });
                        mCmd += drawFxPass(i, `$sw`, `$sh`, `$r_hdl_${i}_x`, `$r_hdl_${i}_y`, 1, fxProfile.sheen, `$anim_local_mode_${i} == 7`, {
                            alphaExpr: `$anim_handle_alpha_${i}`,
                            roundness: 0.96,
                            roundnessBaseExpr: `$sh`,
                            phaseExpr: `$anim_local_phase_${i}`,
                            boostExpr: `$anim_fx_boost_${i}`,
                            resourceKey: 'handle'
                        });
                    }
                }

                if(needsFxWhite) {
                    mCmd += drawFxPass(i, `$abs_w_${i}`, `$abs_h_${i}`, `$abs_x_${i}`, `$abs_y_${i}`, 2, fxProfile.hoverGlow, hoverFxCond);
                    mCmd += drawFxPass(i, `$abs_w_${i}`, `$abs_h_${i}`, `$abs_x_${i}`, `$abs_y_${i}`, 3, fxProfile.selectedGlow, activeFxCond);
                }
                mCmd += endDraw;
                renderQueue.push({ z: m.zIndex || 0, order: i, cmd: mCmd });
            });

            renderQueue.sort((a,b) => (a.z - b.z) || (a.order - b.order)).forEach(item => t+= item.cmd);
            t+=`\n[CustomShaderDraw]\nhs=null\nds=null\ngs=null\ncs=null\nvs=./res/draw_2d.hlsl\nps=./res/draw_2d.hlsl\nblend=ADD SRC_ALPHA INV_SRC_ALPHA\ncull=none\ntopology=triangle_strip\no0=set_viewport bb\nDraw=4,0\nclear=ps-t100\n`;
            if(needsFxWhite) {
                t+=`\n[CustomShaderFx]\nhs=null\nds=null\ngs=null\ncs=null\nvs=./res/draw_2d_fx.hlsl\nps=./res/draw_2d_fx.hlsl\nblend=ADD SRC_ALPHA ONE\ncull=none\ntopology=triangle_strip\no0=set_viewport bb\nDraw=4,0\nclear=ps-t100\n`;
            }
            t = expandElseIfChains(t);
            // 调试：分段落输出 if/endif 计数
            const presentMatch = String(t || '').match(/\[Present\]\n([\s\S]*?)\n\[CustomShaderDraw\]/);
            if(presentMatch) {
                const presentText = presentMatch[1];
                const presentLines = presentText.split('\n');
                // 找各段落边界
                const markers = [
                    'Component Global Animation', 'Local Animation Mode',
                    'Recompute After Clamp', 'Clamp joystick center',
                    'Sync Bindings', 'Recompute After Shared Mirror',
                    'Dependency Logic', 'Render Queue'
                ];
                let sectionStart = 0;
                for(const mk of markers) {
                    const idx = presentText.indexOf(mk, sectionStart);
                    if(idx >= 0) {
                        // 统计从 sectionStart 到 mk 的文字内的 if/endif
                        const seg = presentText.slice(sectionStart, idx).split('\n');
                        let ic=0, ec=0;
                        for(const l of seg) {
                            const tpl=l.trim();
                            if(/^if\b/i.test(tpl)) ic++;
                            else if(/^endif\b/i.test(tpl)) ec++;
                        }
                        if(ic!==0 || ec!==0) console.log(`  [${mk}前] if=${ic} endif=${ec} diff=${ic-ec}`);
                        sectionStart = idx;
                    }
                }
                // 最后一段
                const tail = presentText.slice(sectionStart).split('\n');
                let ic=0, ec=0;
                for(const l of tail) {
                    const tpl=l.trim();
                    if(/^if\b/i.test(tpl)) ic++;
                    else if(/^endif\b/i.test(tpl)) ec++;
                }
                console.log(`  [尾部] if=${ic} endif=${ec} diff=${ic-ec}`);
            }
            try { validatePresentIfBalance(t); } catch(e) { console.warn('Present if/endif未平衡（非致命）:', e.message); }


            return t;
        } catch (e) {
            console.error(e);
            alert("Failed to generate config.\n" + e.message);
            return '';
        }
    }

    async function generateINI() {
        const iniText = buildGeneratedINI();
        if(!iniText) return;
        const rawHash = document.getElementById('char_hash').value || 'c209c22b';
        const safeHash = String(rawHash).replace(/[^A-Za-z0-9_-]+/g, '_');
        const fallbackDownload = () => downloadTextFile(iniText, 'ui_config_' + safeHash + '_' + Date.now() + '.txt');
        try {
            if (ssmtHostBridge.available()) {
                const result = await ssmtHostBridge.request('save-ini', { content: iniText, hash: rawHash });
                alert('INI 已保存到当前工作空间：\n' + result.path);
            } else {
                fallbackDownload();
            }
        } catch (e) {
            if (e && e.message === 'NO_HOST') {
                fallbackDownload();
            } else {
                alert('保存 INI 失败：' + (e && e.message ? e.message : e));
            }
        }
    }

    getPersistentAnimSettings();
    validateShortcutSettings();
    updateGridVis();
    initializeHistorySnapshot();
    refreshResourceWindow(null);
    syncWindowDockButtons();
    void ssmtHostBridge.post({ type: 'ready' });

  // ═══ UIB 公开命名空间(供内联事件处理器引用) ═══
  const __uibExports: Record<string, unknown> = {
  dropTexture,
  handleResourceDragLeave,
  handleResourceDragOver,
  }
  Object.assign((window as unknown as Record<string, unknown>).UIB = {}, __uibExports)
  // 补齐遗漏导出:原 iframe 经典脚本中这些顶层函数天然挂在 window 上,
  // 机械移植为闭包声明后从未导出,导致设置窗/停靠栏上一批按钮静默失效。
  // 这里同时挂回 window(恢复原全局语义)与 window.UIB(前缀事件处理器)。
  const __uibRestoredExports: Record<string, unknown> = {
    addComponent,
    copyPropsToSelection,
    generateAndDownloadAssets,
    generateINI,
    groupSelected,
    loadPreset,
    normalizeShortcutInput,
    redoHistory,
    resetShortcutSettings,
    savePreset,
    toggleAnimationPanel,
    toggleGridSnapAutoY,
    toggleTheme,
    undoHistory,
    ungroupSelected,
    updateAspectRatio,
    updateGridSnapX,
    updateGridSnapY,
    updateUIScale,
    validateShortcutSettings,
  }
  Object.assign(window as unknown as Record<string, unknown>, __uibRestoredExports)
  Object.assign((window as unknown as Record<string, unknown>).UIB as Record<string, unknown>, __uibRestoredExports)
  Object.defineProperty(window.UIB, '_svCapture', { get: () => window._svCapture, configurable: true });
  Object.defineProperty(window.UIB, 'addDepTarget', { get: () => window.addDepTarget, configurable: true });
  Object.defineProperty(window.UIB, 'addGridDepTarget', { get: () => window.addGridDepTarget, configurable: true });
  Object.defineProperty(window.UIB, 'addLinkedSlaveAction', { get: () => window.addLinkedSlaveAction, configurable: true });
  Object.defineProperty(window.UIB, 'addRangeTriggerAction', { get: () => window.addRangeTriggerAction, configurable: true });
  Object.defineProperty(window.UIB, 'addTextRandomBranch', { get: () => window.addTextRandomBranch, configurable: true });
  Object.defineProperty(window.UIB, 'addVarRow', { get: () => window.addVarRow, configurable: true });
  Object.defineProperty(window.UIB, 'delFrame', { get: () => window.delFrame, configurable: true });
  Object.defineProperty(window.UIB, 'removeAccumBinding', { get: () => window.removeAccumBinding, configurable: true });
  Object.defineProperty(window.UIB, 'removeAccumTrigger', { get: () => window.removeAccumTrigger, configurable: true });
  Object.defineProperty(window.UIB, 'removeDepTarget', { get: () => window.removeDepTarget, configurable: true });
  Object.defineProperty(window.UIB, 'removeGridDepTarget', { get: () => window.removeGridDepTarget, configurable: true });
  Object.defineProperty(window.UIB, 'removeLinkedSlave', { get: () => window.removeLinkedSlave, configurable: true });
  Object.defineProperty(window.UIB, 'removeLinkedSlaveAction', { get: () => window.removeLinkedSlaveAction, configurable: true });
  Object.defineProperty(window.UIB, 'removeRangeTrigger', { get: () => window.removeRangeTrigger, configurable: true });
  Object.defineProperty(window.UIB, 'removeRangeTriggerAction', { get: () => window.removeRangeTriggerAction, configurable: true });
  Object.defineProperty(window.UIB, 'removeTextRandomBranch', { get: () => window.removeTextRandomBranch, configurable: true });
  Object.defineProperty(window.UIB, 'removeVar', { get: () => window.removeVar, configurable: true });
  Object.defineProperty(window.UIB, 'removeVarRow', { get: () => window.removeVarRow, configurable: true });
  Object.defineProperty(window.UIB, 'restartDialoguePreview', { get: () => window.restartDialoguePreview, configurable: true });
  Object.defineProperty(window.UIB, 'setDialoguePreviewVariable', { get: () => window.setDialoguePreviewVariable, configurable: true });
  Object.defineProperty(window.UIB, 'setWorkspaceMode', { get: () => window.setWorkspaceMode, configurable: true });
  Object.defineProperty(window.UIB, 'toggleDialoguePreviewPause', { get: () => window.toggleDialoguePreviewPause, configurable: true });
  Object.defineProperty(window.UIB, 'toggleGridTargetEditor', { get: () => window.toggleGridTargetEditor, configurable: true });
  Object.defineProperty(window.UIB, 'updateAccumBinding', { get: () => window.updateAccumBinding, configurable: true });
  Object.defineProperty(window.UIB, 'updateAccumTriggerValue', { get: () => window.updateAccumTriggerValue, configurable: true });
  Object.defineProperty(window.UIB, 'updateAccumTriggerVar', { get: () => window.updateAccumTriggerVar, configurable: true });
  Object.defineProperty(window.UIB, 'updateDefVal', { get: () => window.updateDefVal, configurable: true });
  Object.defineProperty(window.UIB, 'updateDepTargetElse', { get: () => window.updateDepTargetElse, configurable: true });
  Object.defineProperty(window.UIB, 'updateDepTargetInvert', { get: () => window.updateDepTargetInvert, configurable: true });
  Object.defineProperty(window.UIB, 'updateDepTargetVar', { get: () => window.updateDepTargetVar, configurable: true });
  Object.defineProperty(window.UIB, 'updateDialogueVariableInitial', { get: () => window.updateDialogueVariableInitial, configurable: true });
  Object.defineProperty(window.UIB, 'updateFrameVal', { get: () => window.updateFrameVal, configurable: true });
  Object.defineProperty(window.UIB, 'updateGridDefaultOutput', { get: () => window.updateGridDefaultOutput, configurable: true });
  Object.defineProperty(window.UIB, 'updateGridDepTargetElse', { get: () => window.updateGridDepTargetElse, configurable: true });
  Object.defineProperty(window.UIB, 'updateGridDepTargetInvert', { get: () => window.updateGridDepTargetInvert, configurable: true });
  Object.defineProperty(window.UIB, 'updateGridDepTargetVar', { get: () => window.updateGridDepTargetVar, configurable: true });
  Object.defineProperty(window.UIB, 'updateLinkedSlave', { get: () => window.updateLinkedSlave, configurable: true });
  Object.defineProperty(window.UIB, 'updateLinkedSlaveActionValue', { get: () => window.updateLinkedSlaveActionValue, configurable: true });
  Object.defineProperty(window.UIB, 'updateLinkedSlaveActionVar', { get: () => window.updateLinkedSlaveActionVar, configurable: true });
  Object.defineProperty(window.UIB, 'updateLinkedSlaveNum', { get: () => window.updateLinkedSlaveNum, configurable: true });
  Object.defineProperty(window.UIB, 'updateLinkedSlaveRegionPoint', { get: () => window.updateLinkedSlaveRegionPoint, configurable: true });
  Object.defineProperty(window.UIB, 'updateMaxVal', { get: () => window.updateMaxVal, configurable: true });
  Object.defineProperty(window.UIB, 'updateMinVal', { get: () => window.updateMinVal, configurable: true });
  Object.defineProperty(window.UIB, 'updatePath', { get: () => window.updatePath, configurable: true });
  Object.defineProperty(window.UIB, 'updateRangeTriggerActionValue', { get: () => window.updateRangeTriggerActionValue, configurable: true });
  Object.defineProperty(window.UIB, 'updateRangeTriggerActionVar', { get: () => window.updateRangeTriggerActionVar, configurable: true });
  Object.defineProperty(window.UIB, 'updateRangeTriggerNum', { get: () => window.updateRangeTriggerNum, configurable: true });
  Object.defineProperty(window.UIB, 'updateRangeTriggerRegionPoint', { get: () => window.updateRangeTriggerRegionPoint, configurable: true });
  Object.defineProperty(window.UIB, 'updateResourceOpacity', { get: () => window.updateResourceOpacity, configurable: true });
  Object.defineProperty(window.UIB, 'updateTextRandomBranch', { get: () => window.updateTextRandomBranch, configurable: true });
  Object.defineProperty(window.UIB, 'updateVar', { get: () => window.updateVar, configurable: true });
  Object.defineProperty(window.UIB, 'updateVarRow', { get: () => window.updateVarRow, configurable: true });
  Object.defineProperty(window.UIB, 'uploadSeqFrame', { get: () => window.uploadSeqFrame, configurable: true });
  Object.defineProperty(window.UIB, 'uploadTex', { get: () => window.uploadTex, configurable: true });

  return {
    destroy() {
      if (typeof previewClockHandle !== 'undefined' && previewClockHandle) cancelAnimationFrame(previewClockHandle)
      if (typeof dialogueRuntimeFrame !== 'undefined' && dialogueRuntimeFrame) cancelAnimationFrame(dialogueRuntimeFrame)
      for (const [type, fn, opts] of __windowListeners) window.removeEventListener(type, fn, opts)
      for (const [type, fn, opts] of __documentListeners) document.removeEventListener(type, fn, opts)
      __windowListeners.length = 0
      __documentListeners.length = 0
    },
  }
}
