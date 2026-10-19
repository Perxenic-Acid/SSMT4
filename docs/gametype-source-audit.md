# GameType（数据类型）来源审计：随包一套与用户配置一套，为什么"两边同时生效"

- 审计时间：2026-10-19
- 仓库：`E:\代码\SSMT4-Alpha-main` @ `af3e687`（main，任务一刚快进到 upstream 最新）
- 现场对象：安装目录 `K:\ssmt4_1`（ssmt4.exe 16,557,568B，2026-10-16 23:42 构建）、用户配置 `%LOCALAPPDATA%\SSMT4GlobalConfigs`、游戏预设 `Games\<preset>\Config.json`
- 方式：**只读**源码 + **只读**磁盘。审计全程 `git status --short | wc -l` = 0，未修改任何代码或 git 状态
- 提问原话：「现在提取所使用的数据类型应当全部都是外部的了，他还有内部的数据类型吗？为什么他两边同时在生效？这是什么情况？」
- ⚠ 本文描述的是**修复前**的状态（§1–§5）。第一轮修复（交付清单）见 §6，但该模型已被用户否决并再次重写，**最终形态见 §7「数据类型只跟程序走」**——§6 仅作为过程记录保留。

---

## 0. 结论速览

| # | 现象 | 事实 | 关键证据 |
|---|---|---|---|
| 1 | 「提取用的数据类型都是外部的」 | **对**。后端提取链的唯一读取源是 `%LOCALAPPDATA%\SSMT4GlobalConfigs\GameType\<游戏名>\*.json`，全仓只有 1 个读取入口 | `src-tauri/src/gametype/mod.rs:137-148`（`get_game_type_list` 是唯一入口）、`src-tauri/src/config/path_manager.rs:121-125` |
| 2 | 「他还有内部的数据类型吗」 | **有，但不是当年那套代码注册表**。硬编码注册表（22 个 `type_*.rs`）已在 `d6b5d50` 删除，`src-tauri/src/gametype/` 现在只剩 `mod.rs`。仍存在的「内部那套」是**安装包里的 `resources\GameType\` 随包副本** | `d6b5d50` 的删除清单（22 个文件）；`src-tauri/tauri.conf.json` 的 `bundle.resources` 含 `resources/GameType/**/*` |
| 3 | 「为什么两边同时在生效」 | 因为**每次启动把「内部（随包）」覆盖式同步进「外部（用户目录）」，只增不删**。运行时只读外部；内部那套通过同步管道间接生效，此外**前端 3D 预览还直接读安装目录**，那才是字面意义上的"两边同时" | `src-tauri/src/lib.rs:124-133` → `src-tauri/src/gametype/mod.rs:19-29` / `:31-58`；`src/views/ModsManagement/Mod3DPreview.vue:441-447` |
| 4 | 同名冲突谁赢 | **随包的赢**（`fs::copy` 覆盖同名）；只有随包没有的名字用户新增才保留。两侧**不是运行时合并、也不是兜底**，是"启动时单向覆盖 + 文件级并集" | `src-tauri/src/gametype/mod.rs:47-53`（`fs::copy` 覆盖）、实测 install⊆user 逐字节 0 差异 |
| 5 | 顺带查出的真问题 | 用户目录里至今仍在加载 **9 个仓库已废弃的数据类型**（IdentityV 5 个 + IdentityV2 4 个），来源是**陈旧的构建产物副本**（`target/debug/resources/GameType`）被同步进去后**永不清理** | 实测目录对比；`git log --diff-filter=D` 的 `75f00ac` / `d6b5d50` |

---

## 1. 运行时到底从哪些地方读 GameType 定义

### 1.1 后端提取链：唯一读取入口是「用户配置目录」

`src-tauri/src/gametype/mod.rs:137-148`，整个后端只有这一个函数去拿类型定义：

```rust
pub fn get_game_type_list(game_name: &str) -> Result<Vec<D3D11GameType>, String> {
    let root = runtime_gametype_folder();          // L138 → PathManager::ssmt_gametype_folder()
    let folder = external_game_type_folder(game_name)
        .ok_or_else(|| format!("GameType folder not found for {} under {}", game_name, root.display()))?;
    load_game_type_list_from_folder(&folder, game_name)
}
```

`runtime_gametype_folder()` 就是用户配置目录（`src-tauri/src/config/path_manager.rs:121-125`）：

```rust
/// 获取 GameType 文件夹路径（位于 SSMT4GlobalConfigs 下）
/// 与 Games 文件夹一样位于 SSMT4GlobalConfigs 下，是运行时唯一的数据类型读取来源
pub fn ssmt_gametype_folder() -> PathBuf {
    Self::ssmt_global_config_folder().join("GameType")   // %LOCALAPPDATA%\SSMT4GlobalConfigs\GameType
}
```

### 1.2 五个候选位置的真实角色

| 位置 | 绝对/相对路径 | 角色 | 证据 |
|---|---|---|---|
| **① 用户配置目录** | `%LOCALAPPDATA%\SSMT4GlobalConfigs\GameType\<游戏>\*.json` | **运行时唯一后端读取源** | `path_manager.rs:121-125`；`gametype/mod.rs:8-10`、`:137-148` |
| **② 安装目录随包副本** | `<exe目录>\resources\GameType\`（或 `<exe目录>\..\resources\GameType`） | 首次运行的**初始来源**；每次启动同步进①；**前端 3D 预览直接读它** | `path_manager.rs:63-86`、`:93-97`；`gametype/mod.rs:19-29`；`Mod3DPreview.vue:445` |
| **③ 开发态源码树** | `src-tauri/resources/GameType`（`current_dir()` 回退） | 只在开发态、且 exe 旁边没有 `resources` 时成为② | `path_manager.rs:79-85` |
| **④ 构建产物副本** | `src-tauri/target/{debug,release}/resources/GameType` | dev 运行时它就是②，**会把陈旧内容同步进①** | `path_manager.rs:66-70`；实测它含 5 个仓库已删的 IdentityV json |
| **⑤ 游戏预设字段** | `Games\<preset>\Config.json` 的 `GameTypeName` | **未见消费者（未证实）** | 见 §1.6 |

### 1.3 消费方：17 个提取后端，全部按「路径最后一段」取名字

每个后端都是同一句式（`join` 的名字就是游戏标识）：

| 后端 | 位置 | 传入的名字 |
|---|---|---|
| WWMI | `src-tauri/src/common/frame_analysis/frameanalysis_base.rs:55`（唯一调用方 `src-tauri/src/extract_new/wwmi.rs:30-35`） | `"WWMI"` |
| ZZMI | `src-tauri/src/extract_new/zzmi.rs:488-489` | `"ZZMI"` |
| ZZMIDX12 | `src-tauri/src/extract_new/zzmi_dx12.rs:1583-1584`、`:1621-1622` | `"ZZMIDX12"` |
| GIMI | `src-tauri/src/extract_new/gimi.rs:388-389` | `"GIMI"` |
| HIMI | `src-tauri/src/extract_new/himi.rs:461-462` | `"HIMI"` |
| SRMI | `src-tauri/src/extract_new/srmi.rs:1346-1347` | `"SRMI"` |
| EFMI | `src-tauri/src/extract_new/efmi3.rs:202-203` | `"EFMI"` |
| CAMI | `src-tauri/src/extract_new/cami.rs:197-198` | `"CAMI"` |
| APMI | `src-tauri/src/extract_new/apmi2.rs:386-387` | `"APMI"` |
| AILIMIT | `src-tauri/src/extract_new/ailimit.rs:399-400` | `"AILIMIT"` |
| GF2 | `src-tauri/src/extract_new/gf2.rs:350-351` | `"GF2"` |
| NTEMI | `src-tauri/src/extract_new/ntemi.rs:934-935` | `"NTEMI"` |
| YYSLS | `src-tauri/src/extract_new/yysls.rs:509-510` | `"YYSLS"` |
| SnowBreak | `src-tauri/src/extract_new/snowbreak.rs:187-188` | `"SnowBreak"` |
| Naraka | `src-tauri/src/extract_new/naraka.rs:363-364` | `"Naraka"` |
| NarakaM | `src-tauri/src/extract_new/narakam.rs:382-383` | `"NarakaM"` |
| IdentityV | `src-tauri/src/extract_new/identityv.rs:324-325` | `"IdentityV"` |

**关键机制（很容易被误读）**：`D3D11GameTypeLv2::new(game_type_folder: impl AsRef<Path>)` **并不使用传入的路径**，只用它的最后一段当游戏名（`src-tauri/src/common/d3d11_gametype_lv2.rs:41-55`）：

```rust
pub fn new(game_type_folder: impl AsRef<Path>) -> Result<Self, String> {
    let folder_path = game_type_folder.as_ref();
    let game_name = folder_path.file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| format!("Unable to infer game name from path: {}", folder_path.display()))?;
    Self::from_game_name(game_name)     // L53 → get_game_type_list(name)
}
```

所以 `PathManager::ssmt_gametype_folder().join("GIMI")` 里的 `join("GIMI")` 只是**把游戏名传进去的写法**；真正落地的目录由 `external_game_type_folder()` 在用户配置根下重新解析。

### 1.4 目录解析细节

`src-tauri/src/gametype/mod.rs:61-87`：

- 名字先 `trim` + `to_ascii_uppercase`（L62）
- **`IDENTITYV` 硬特例**：直接 `root.join("IdentityV")`（L66-68）
- 否则扫描根目录，取 `eq_ignore_ascii_case` 匹配的子目录（L70-83）——所以 `%LOCALAPPDATA%\...\GameType\IDENTITYV`（全大写）与 `IdentityV` 会命中同一个物理目录（Windows 不区分大小写）
- 都没有才兜底 `root.join(normalized)`，存在才返回（L85-86）

`load_game_type_list_from_folder()`（`:90-135`）只取**该目录第一层**的 `*.json`（不递归，扩展名大小写不敏感，L105-112），排序后逐个解析（L115-119）；目录里没有 json 直接报错（L129-133）。**目录里有多少 json 就加载多少，没有白名单、没有版本校验**——这是 §3.4 dead data 能被加载的直接原因。

### 1.5 启动时「内部 → 外部」的同步

`src-tauri/src/lib.rs:124-133`：

```rust
match gametype::sync_bundled_gametype_to_config() {
    Ok(copied) => println!("[GameType] Synced {copied} bundled GameType file(s) to user config."),
    Err(error) => eprintln!("[GameType] Failed to sync bundled GameType folder: {error}"),
}
```

失败**只打日志、不阻断启动**（这本身是合理的降级设计，但代价是"同步失败"完全静默）。

`src-tauri/src/gametype/mod.rs:19-58` 的实现：source = 随包，target = 用户目录，`fs::copy` **覆盖同名**，**从不删除目标端任何文件**。

### 1.6 另外两处「看起来是来源、其实不是 / 未证实」的地方

- **游戏预设的 `GameTypeName`**：实测 `%LOCALAPPDATA%\SSMT4GlobalConfigs\Games\<preset>\Config.json` 里确实有这个字段：APMI `None`、EFMI `'EFMI'`、GF2 `None`、GIMI `'GIMI'`、HIMI `'HIMI'`、IdentityV2 `None`、NTEMI `'EFMI'`、SRMI `'SRMI'`、WWMI `'WWMI'`、WuWa `'WuWa'`、**ZZMI `'HIMI'`**、**ZZMIDX12 `'HIMI'`**。但前端 `src/` 对 `GameTypeName` grep **0 命中**，Rust 侧只有 `D3D11GameType` 自身的同名字段（`src-tauri/src/common/d3d11_gametype.rs:12-13`、`:86-99`）——**未证实它还有消费者**。注意它与后端硬编码的 `join("ZZMI")` / `join("ZZMIDX12")`（`zzmi.rs:489`、`zzmi_dx12.rs:1584/1622`）**矛盾**：预设说 HIMI，提取器读 ZZMI/ZZMIDX12。
- **`src-tauri/src/constants/gametype*.rs`**：见 §2.2，它们是**字段词表**（Category/ElementName/ExtractSlot/Technique/Format 的字符串常量），不是数据类型定义表，不参与"读哪个目录"。

---

## 2. 还有没有「内部」的那套？

分四层回答，因为"内部"在这个项目里至少有三个不同含义。

### 2.1 曾经的「内部」= 代码里的硬编码注册表：**已经删除**

`git log --diff-filter=D --name-only` 显示：

- `d6b5d50`（"添加一个第五人格数据类型，修复第五人格数据类型无法使用的BUG"）一次性删除了 **全部 22 个** `src-tauri/src/gametype/type_*.rs`：`type_ailimit / apmi / doav / efmi / gf2 / gimi / himi / hok / identityv2 / miside / naraka / narakam / neirr / nioh2 / ntemi / snowbreak / srmi / theoutcast / wwmi / yysls / zzmi / zzmidx12`
- `75f00ac`（"删除已废弃的 IdentityV 死数据（旧类型与 JSON 定义）"）删除了 `type_identityv.rs` 与 6 个 IdentityV JSON

**现在 `src-tauri/src/gametype/` 只剩 `mod.rs`**，代码里没有任何一处"内置类型表"。所以"内部的数据类型"在**代码层面**确实已经不存在了。

### 2.2 `constants/gametype*.rs`：词表，且大部分已经是死代码

| 文件 | 内容 | 运行时引用数 |
|---|---|---|
| `constants/gametype_category_name.rs` | `CategoryName`（Blend/Color/Normal/Position/Tangent/Texcoord/trianglelist/Vector/BLENDINDICES/BLENDWEIGHTS） | `CategoryName::` **0 处**（死代码） |
| `constants/gametype_element_name.rs` | `ElementName`（BINORMAL/BLENDINDICES/BLENDWEIGHT(S)/COLOR/NORMAL/POSITION/TANGENT/TEXCOORD/BITANGENT） | `ElementName::` 2 处（`common/migoto_model.rs`、`common/d3d11_gametype.rs`） |
| `constants/gametype_extract_slot.rs` | `ExtractSlot`（cs-t0..t4 / vb0..vb4 / vs-t0..t8） | `ExtractSlot::` **0 处**（死代码） |
| `constants/gametype_extract_technique.rs` | `ExtractTechnique`（pointlist/trianglelist） | `ExtractTechnique::` 6 处（frame_analysis ×2、fmt_file.rs、d3d11_gametype_lv2.rs、d3d11_gametype.rs、commands/extract_model.rs） |
| `constants/gametype_format.rs` | `DxgiFormat`（21 个格式常量） | `DxgiFormat::` **0 处**（死代码） |
| `constants/gametype.rs` | 仅 re-export | — |

它们是**字符串常量与本项目的类型定义对齐用的**，不构成第二套数据来源。

### 2.3 没有 `include_str!` / 内嵌资源

全仓 grep `include_str!` / `include_bytes!` 命中的都是插件资产 json，**没有**任何把 GameType JSON 编进二进制的写法。

### 2.4 `from_parts` 只是测试辅助

`D3D11GameType::from_parts` 全仓只有 1 处调用：`src-tauri/src/extract_new/srmi.rs:2637`，位于 `#[cfg(test)]` 模块内（构造 `"GPU_AlternateT8Interpretation"`），**非运行时路径**。

### 2.5 真正"还在"的内部那套 = **安装包里的随包副本**

这才是用户感觉到的"内部数据类型"：

- `src-tauri/resources/GameType/`（332 个 json，26 个游戏目录）经 `src-tauri/tauri.conf.json` 的 `bundle.resources`（含 `resources/GameType/**/*`）打进安装包
- 装完后就是 `K:\ssmt4_1\resources\GameType\`（336 个 json）
- 每次启动被覆盖式同步进 `%LOCALAPPDATA%\SSMT4GlobalConfigs\GameType\`
- 其自带 `README.md` 明确写着自身定位："single source of truth"、启动时同步、同名覆盖、用户新增保留

它随包分发、跟随版本更新，**但它不是运行时读取源**——它是"初始值 + 补丁源"。

### 2.6 还有第 4 份副本（意外发现）：构建产物

`src-tauri/target/debug/resources/GameType`（330 个 json）与 `target/release/resources/GameType`（326 个 json）。

开发态 `path_manager.rs:63-86` 的三级判定里，`current_exe().parent()/resources` 优先，所以 `bun tauri dev` 跑起来时"随包副本"= **这份陈旧的构建产物**，它会把老内容同步进用户目录。见 §3.4 的实锤。

---

## 3. 为什么「两边同时在生效」

### 3.1 两套是什么

- **A（内部/随包）**：安装目录 `resources\GameType\`
- **B（外部/用户）**：`%LOCALAPPDATA%\SSMT4GlobalConfigs\GameType\`

### 3.2 关系不是"合并"，也不是"兜底"，而是**启动时单向覆盖式同步**

`src-tauri/src/gametype/mod.rs:31-58`：递归遍历 A，对每个文件 `fs::copy(源, 目标)`——**同名覆盖**，**只增不删**。即：

- 同名文件：**A 赢**（B 的版本被每次启动抹掉）
- 只在 A：复制进 B ⇒ 生效
- 只在 B：**原样保留** ⇒ 生效（这就是"用户新增类型"的官方支持路径，`README.md` 与测试 `sync_folder_copies_overwrites_and_preserves_user_files` 都确认）

所以 B 的最终内容 = **A ∪ B₀**（后写覆盖），运行时再把这整个并集读成类型表。

### 3.3 现场实测（2026-10-19，SHA-256，路径小写归一 + 换行归一化）

| 树 | 文件数 |
|---|---|
| repo `src-tauri/resources/GameType` | 332 |
| install `K:\ssmt4_1\resources\GameType` | 336 |
| `src-tauri/target/debug/resources/GameType` | 330 |
| `src-tauri/target/release/resources/GameType` | 326 |
| **user `%LOCALAPPDATA%\SSMT4GlobalConfigs\GameType`** | **348** |

- **install vs user：同名 336 个，逐字节差异 0**；`只在 install` = 0；`只在 user` = 12
  → **同步确实执行过且是"覆盖"语义**（336 个文件全被抹成安装包版本），用户目录是安装目录的**逐字节超集**
- **repo vs user：同名 332，内容差异 0**（唯 6 个"差异"是 CRLF/LF 换行差异：`gametype-analysis/SKILL.md`、`agents/openai.yaml`、`references/PROJECT-CONVENTIONS.md`、`scripts/*.py` 三个）
- **repo vs install：同名 330，内容差异 0**；`只在 repo` 2（`himi/gpu_p12_n12_c4_ta16_bw16_bi16_t4_t1-4_.json`、`..._t2-4_.json`）；`只在 install` 6（2 个 `__pycache__/*.pyc` + `identityv2/` 4 个 json）

结论：**两侧数据 99% 完全一致**——这就是"看着像两边同时生效"的直接观感来源：它们本来就是同一批文件的两次落盘。

### 3.4 实锤：9 个已废弃的类型至今仍在被加载

用户目录比安装目录多出的 12 个文件里，有 **7 个是从陈旧构建产物灌进去的**：

| 文件 | 仓库状态 | 从哪来 |
|---|---|---|
| `identityv/CPU_P12_N12_TA12_C4_T8_T1-8_T2-8_.json` | `75f00ac` 已删除 | `target/debug/resources/GameType/identityv/` |
| `identityv/CPU_P12_N12_TA12_C4_T8_T1-8_vb0vb1.json` | `75f00ac` 已删除 | 同上 |
| `identityv/GPU_P12_N12_TA12_C4_T8_T1-8_BW16_BI8_.json` | `75f00ac` 已删除 | 同上 |
| `identityv/GPU_P12_N12_TA12_C4_T8_T1-8_T2-8_BW16_BI8_.json` | `75f00ac` 已删除 | 同上 |
| `identityv/GPU_P12_N12_TA12_C4_T8_T1-8_T2-8_T3-8_BW16_BI8_.json` | `75f00ac` 已删除 | 同上 |
| `himi/gpu_p12_n12_c4_ta16_bw16_bi16_t4_t1-4_.json` | 仓库有（仓库 332，安装包 336 里缺这两个） | `target/debug` |
| `himi/..._t2-4_.json` | 同上 | `target/debug` |
| `identityv2/` 4 个 json | 目录在 `9dc3ebb` 加入、`d6b5d50` 删除 | 安装包残留（目录 mtime 早于本次安装） |

**加载链条**：`identityv.rs:325` `join("IdentityV")` → normalize 成 `IDENTITYV` → `external_game_type_folder` 的 IDENTITYV 特例（`mod.rs:66-68`）落到用户目录的 `IDENTITYV\` → `load_game_type_list_from_folder` 把该目录**全部 10 个 json** 载入（`mod.rs:90-135`，无白名单）。

于是：

- 代码里**已经删掉**的 5 个 IdentityV 老类型，**至今还在可选类型列表里**
- `IdentityV2` 这个**连预设都删了**的整目录（用户 `Games/` 里还留着 `IdentityV2/` 预设，而随包 `resources/Games` 里没有），只要有人调用 `get_game_type_list("IdentityV2")` 就仍然读得到

这不是"两套来源打架"，而是**"只增不删"的同步语义 + 无版本/白名单校验**共同造成的**废弃数据复活**。

### 3.5 真正字面意义上的"两边同时生效"：前端读安装目录

`src/views/ModsManagement/Mod3DPreview.vue:441-447`：

```ts
const resources = await resourceDir();
paths.push(await join(resources, 'GameType', 'GIMI'));
paths.push(await join(resources, 'resources', 'GameType', 'GIMI'));
// 开发回退
paths.push('src-tauri/resources/GameType/GIMI', 'resources/GameType/GIMI');
```

它逐个数 `GameType/GIMI/*.json` 的 `D3D11ElementList` 来算 slot/stride 布局（`:450-465`）——**只读安装目录/源码树，从不读用户配置目录**。

所以准确的景象是：

- **后端提取**：只读外部（用户配置目录）
- **前端 3D 预览**：只读内部（安装目录）
- **两者不一致时**：用户在用户目录新增/修改 GIMI 类型 → 后端认、前端不认（预览的 slot/stride 会与提取结果对不上）

如果用户看到的"两边同时在生效"指的就是这个，那它是**真的**，而且是**潜在 bug**，不是设计。

### 3.6 同步源在 dev 下是构建产物，不是源码树

`path_manager.rs:63-86` 判定顺序：`current_exe().parent()/resources` → `exe_dir.parent()/resources` → `current_dir()/src-tauri/resources`。

`bun tauri dev` 的 exe 在 `target/debug/`，旁边正好有 `target/debug/resources/` ⇒ ②选中它，源码树（③）根本不会被考虑到。而 `target/` 是**不会被清理的构建产物**，于是老 json 从这里持续回灌。§3.4 的 5 个 IdentityV 死类型就是这么来的。

---

## 4. 结论：内部那套还有没有存在必要？

| 对象 | 还有必要吗 | 说明 |
|---|---|---|
| 代码里的硬编码注册表（22 个 `type_*.rs`） | **没有，已经删掉了** | 正确决策。删掉不会断任何东西：`gametype/mod.rs` 已不含任何内置类型 |
| 随包 `resources/GameType/` | **有，必须留** | 首次启动/离线/老用户升级只有靠它才有类型可用；打包清单 `bundle.resources` 依赖它 |
| `constants/gametype*.rs` | **部分可清理** | `CategoryName`、`DxgiFormat`、`ExtractSlot` 当前 0 引用（死代码）；`ElementName`、`ExtractTechnique` 在用。清理属于独立的低风险改动，本次不动 |
| `target/{debug,release}/resources/GameType` | **不是"来源"，是污染源** | 不该被当成同步源。见下 |
| 游戏预设的 `GameTypeName` | **未证实** | 无消费者；且与后端硬编码名字矛盾（ZZMI/ZZMIDX12 写着 HIMI） |

### 删掉/改动会不会断？

- **老配置**：不会。用户目录里的类型文件继续被读，同步只覆盖同名、从不删。
- **老版本**：不会。老版本自己带随包副本，同步逻辑与现在一致。
- **离线场景**：不会，反而更强——同步是纯本地文件复制，不联网。
- **首次启动**：**会断**——如果没有随包副本，`sync_bundled_gametype_to_config()` 直接 `Err("Bundled GameType folder not found: ...")`，且用户目录空 ⇒ `get_game_type_list()` 对每个游戏都报 `GameType folder not found for X under ...`，提取全废。**所以随包那套不能删。**

### 建议（按优先级，本次未实施）

1. **【中】修同步源判定**：dev 态别让 `target/*/resources` 当同步源（或同步时跳过 `target` 路径），并考虑把"随包副本"与"源码树"的优先级写清楚。这可止住"陈旧产物回灌"。
2. **【中】给同步加"随包已删除的类型"清理策略**：至少对"随包曾经有、现在没有"的文件（需要一份 manifest）做删除或移入 `_deprecated/`。当前"只增不删"会让废弃类型永远活着。若担心误删用户新增，可只清理"随包 manifest 里记录过"的名字。
3. **【低】统一前端与后端的类型来源**：`Mod3DPreview.vue` 应改读用户配置目录（或至少优先用户目录、回退安装目录），否则用户新增类型在预览里不生效。
4. **【低】清理死常量**：`CategoryName` / `DxgiFormat` / `ExtractSlot` 无引用，可删或标注。
5. **【低】处理实例数据**：用户目录里的 9 个废弃类型（IdentityV ×5、IdentityV2 ×4）与用户 `Games/IdentityV2/` 预设是历史残留，可提示用户删除（属用户数据，不建议程序擅自删）。
6. **【信息】确认 `Games/*/Config.json` 的 `GameTypeName` 是否还有意义**：无消费者且与硬编码矛盾，值得明确废掉或统一。

---

## 5. 未证实项

1. `Games/<preset>/Config.json` 的 `GameTypeName` 是否还有运行时消费者——前端 0 命中，Rust 侧未见读取该字段的代码。**未证实**（也可能在其他 crate / 动态读取路径里）。
2. 用户在真实使用中感知的"两边同时在生效"具体指哪一种：是 §3.2 的"随包内容出现在用户目录"，还是 §3.5 的"前端读安装目录 / 后端读用户目录"。报告对两者都给了结论。
3. `target/debug` 那 5 个 IdentityV 死类型具体是哪一次 `bun tauri dev` 同步进去的（无日志留存，无法定位到具体时间）。
4. 安装包内 `identityv2/` 目录 mtime 为 Aug 8，与本次安装时间不符——推断为打包时保留了陈旧文件（**未证实**）。

---

## 附录：复现命令

```bash
# 1) 五棵树的存在与数量
ls -d src-tauri/resources/GameType K:/ssmt4_1/resources/GameType \
      src-tauri/target/debug/resources/GameType \
      src-tauri/target/release/resources/GameType \
      "$LOCALAPPDATA/SSMT4GlobalConfigs/GameType"

# 2) 读取入口与同步逻辑
sed -n '1,150p' src-tauri/src/gametype/mod.rs
sed -n '110,130p' src-tauri/src/config/path_manager.rs
sed -n '120,135p' src-tauri/src/lib.rs

# 3) 消费方
grep -rn "ssmt_gametype_folder" src-tauri/src --include=*.rs

# 4) 硬编码注册表的删除历史
git log --diff-filter=D --name-only -- 'src-tauri/src/gametype/type_*.rs'
git log --oneline --all -- 'src-tauri/resources/GameType/IdentityV2*'

# 5) 四棵树集合差异 + 内容比对（脚本留在 E:\代码\_acp_tmp\_cmp_gametype.py）
/c/Python311/python E:/代码/_acp_tmp/_cmp_gametype.py
```

> 注：Windows 下给原生程序传 MSYS 风格路径（`/e/...`）会被改写成 `E:\e\...`；用 Python 比对时请 `cd` 后用相对路径或反斜杠路径。

---

## 6. 修复：外部唯一可信来源（本次实施）

> 决策来自用户：**「将外部那一套作为唯一可信来源，内部那一套直接删掉」**，随后澄清「内部的那一套指的是**被打包进 EXE 程序的那一套**」。

### 6.1 先查清"打包进 EXE"这件事（结论：exe 里没有任何类型数据）

在安装版 `K:\ssmt4_1\ssmt4.exe`（16,557,568 B，2026-10-16 构建）里直接搜字符串：

| 搜索串 | 命中 | 说明 |
|---|---|---|
| `CPU_ATTRIBUTE-16_ATTRIBUTE1-8_ATTRIBUTE2-16_ATTRIBUTE3-4_`（CAMI 专有类型名，只存在于 JSON 文件） | **0** | 类型 JSON 若被编进 exe 必然命中 |
| `GPU_P12_N12_TA16_C4_T8_BW16_BI16_`（GIMI 专有类型名，只存在于 JSON 文件） | **0** | 同上 |
| `include_dir`（整目录内嵌宏） | **0** | 没有整目录内嵌 |
| `D3D11ElementList` | 1 | Rust 源码里的 serde 字段名，预期命中 |
| `SemanticName` | 2 | 同上 |

配合 §1 的源码侧证据（全仓无 `include_str!`/`include_bytes!` 内嵌 GameType；22 个硬编码注册表 `src-tauri/src/gametype/type_*.rs` 已在 **`d6b5d50` 2026-08-09 14:37:31** 删除，比这次安装 10-16 早两个月）：

**结论**：`ssmt4.exe` 内不含任何数据类型定义。所谓"打包进 EXE 的那一套"，物理上是**紧挨 exe 的文件副本** `K:\ssmt4_1\resources\GameType`（由安装包按 `src-tauri/tauri.conf.json:50` 的 `bundle.resources` 铺到安装目录）。它随安装程序一起来，所以体感像"程序自带的那一套"。

### 6.2 根因

修复前的 `sync_folder()`（原 `src-tauri/src/gametype/mod.rs:31-58`）**每次启动无条件覆盖同名文件**、只增不删 ⇒ 用户在外部删掉错误的数据类型，下次启动又被随包副本复活。所以"两边同时在生效"不是合并、也不是兜底，而是**覆盖式重灌**：外部是被读的那一套，随包副本是每次启动都往回灌的那一套。

### 6.3 新交付模型

`sync_bundled_gametype_to_config()` → `deliver_bundled_gametype_to_config()`（`src-tauri/src/gametype/mod.rs:67`）。随包副本降级为**首次交付源**，用 `%LOCALAPPDATA%\SSMT4GlobalConfigs\GameType\.delivery.json`（`mod.rs:33`）记录"程序交付过哪些文件、交付时的 sha256 指纹"，按状态决策：

| 状态 | 判定 | 动作 |
|---|---|---|
| 首次交付 | 外部没有 + 清单里没有 | 写入并记入清单 |
| 随包更新 | 外部有 + 清单指纹 == 当前内容 + 随包内容不同 | 覆盖（版本升级仍能下发类型修正） |
| 用户改过 | 外部有 + 清单指纹 != 当前内容 | **保留用户版本**，永不覆盖 |
| 用户删过 | 外部没有 + 清单里有记录 | **不补回**，永久尊重删除 |
| 用户自建 | 外部有 + 清单里没有记录 | 不触碰（内容与随包一致时只"认领"进清单，不改文件） |

实现要点：

- 交付前后都比较清单内容，没有变化就不写盘，避免每次启动都改用户目录。
- `.delivery.json` 是**文件**不是目录，`external_game_type_folder()` 扫描根目录时只认目录（`mod.rs:246`），不会干扰类型发现；`load_game_type_list_from_folder()` 只读游戏目录第一层的 `*.json`，读不到它。
- 删掉 `.delivery.json` = 恢复出厂：下次启动会重新交付一遍，已存在且内容一致的文件只是被重新"认领"进清单。
- 运行时读取链没有变：`get_game_type_list()`（`mod.rs:311`）依旧只读用户配置目录。

### 6.4 改动清单

| 文件 | 改动 |
|---|---|
| `src-tauri/src/gametype/mod.rs` | 删 `sync_bundled_gametype_to_config`/`sync_folder`；新增 `DeliveryManifest`(:40)、`DeliveryReport`(:48)、`deliver_bundled_gametype_to_config`(:67)、`deliver_folder`(:112)、`deliver_file`(:142)、`write_delivered_file`(:205)、`content_hash`(:222)；测试由 1 个"覆盖式同步"用例改为 5 个交付语义用例（:421-522） |
| `src-tauri/src/lib.rs:127` | 启动调用改为 `deliver_bundled_gametype_to_config()`，日志输出交付统计（新增/更新/未变/保留用户修改/保留用户删除/用户自建） |
| `src/views/ModsManagement/Mod3DPreview.vue` | 头部 `:9` 引入 `GlobalConfig`；`:442-450` GIMI 布局改为**优先读用户配置目录**，随包/源码树只作回退 |
| `src-tauri/resources/GameType/README.md` | "每次启动同步并覆盖"改为交付语义说明（四态规则 + `.delivery.json`） |

### 6.5 验证

`cargo test gametype`（2026-10-19，工作目录 `src-tauri/`）：

    10 passed; 0 failed; 0 ignored; 0 measured; 252 filtered out

与本次改动直接相关的 5 条：

| 用例 | 断言 | 结果 |
|---|---|---|
| `delivery_seeds_missing_files_once` | 空目录首次交付 2 个文件；第二次启动 `unchanged=2`，不再改动任何文件 | ok |
| `delivery_never_restores_user_deleted_file` | 用户删掉外部文件后再交付：`kept_user_deleted=1`、`delivered=0`、文件仍然不存在 | ok |
| `delivery_keeps_user_modified_file` | 用户改过的文件，即使随包也有新内容也保留用户版本 | ok |
| `delivery_updates_file_user_did_not_touch` | 用户没动过的文件随包更新时被覆盖（`updated=1`） | ok |
| `delivery_keeps_user_owned_files` | 清单外的用户文件（含同名不同内容）既不被覆盖也不被删除 | ok |

另两条既有用例继续通过：`all_game_type_folders_load_from_json`、`identityv_loads_user_added_split2_type`。

`cargo check --lib`：`Finished dev profile`，无本次改动引入的警告（只有一条既有的 build script 提示 `Using DirectXTex from vendored native/directxtex`）。

前端 `bunx vue-tsc --noEmit`：exit=0，无类型错误。

**环境注记（与本次改动无关的既有问题）**：`cargo test` 的 build script 要求 `src-tauri/resources/d3d11.dll` 存在（`src-tauri/tauri.conf.json:50` 的 `bundle.resources` 声明），而当前工作区**没有这个文件**：它被 `.gitignore:66` 忽略、不在版本控制内，也不在任务一备份里——任务一备份用的是"未跟踪且未忽略"清单，当时的 `.gitignore`（`0b2c4d1` 版本）还**没有**这条忽略规则，如果文件当时存在就必然出现在清单里，因此它在本轮任何操作之前就已缺失。本次验证用 `TAURI_CONFIG='{"bundle":{"resources":[]}}'` 临时覆盖资源清单完成编译，**没有改动任何仓库文件**；正常 `bun tauri dev` 需要先用 `runtime` 的 `debug.ps1` 生成并部署该 DLL。

### 6.6 仍然保留的随包副本，以及为什么不能一起删

按用户要求"删掉内部那套"，物理上能做的是**从安装包里彻底移除 `resources/GameType`**（`tauri.conf.json` 的 `bundle.resources` 条目）。本次**没有**这样做，原因：

- 全新安装、换机、用户清空外部目录时，`get_game_type_list()` 会对每个游戏返回 `GameType folder not found for X under ...`（`mod.rs:315`），提取与 3D 预览全部失效。
- 全仓**没有任何网络下发数据类型的渠道**（命令层/MCP 里 `gametype` 0 命中，`tauri_plugin_http` 与类型无关）。

所以本次把随包那一套降级为"你没有的时候给你一份"，它**不再能覆盖、也不再能复活外部目录里的任何东西**。若要彻底让安装包不带类型，需要先定好替代供给（Release 资产 / 插件包 / 首次运行引导），属独立任务。

---

## 7. 最终形态：数据类型只跟程序走（取代 §6）

§6 的"交付清单 + 四态同步"模型被用户否决。用户的判断是：**数据类型是程序内容，不是用户配置**——它应该像其他资源一样"随安装覆盖"，放用户目录只会带来卸载残留与重装不覆盖（用户原话见下）。最终实现是：**类型库只有一份，就是装机目录里那一份；运行时只读它；用户配置目录彻底退出类型链路。**

### 7.1 用户决策链（原话）

- m00451：「以安装目录为准」
- m00455：「用户目录不要再存了，存它干嘛呢？没什么用啊」
- m00459：「为什么会存在用户目录那一份啊？我卸载的时候卸不干净，然后安装的时候它又不会被覆盖」
- m00463：「我远程项目追加数据类型，追加到的也是安装目录里面那一份啊。也就是说，如果我以后更新版本追加了数据类型，用户目录就永远读不到啊」
- m00464：「还有找数据类型的那个技能。最后生成的数据类型也应该是安装目录啊」

这四条合起来否定了"用户目录是工作副本"的整个前提：追加数据类型的写点在安装目录，读点却在用户目录，两边永远不会合流。

### 7.2 最终语义

| 项 | 形态 |
| --- | --- |
| 类型库唯一位置 | `<安装目录>/resources/GameType/<游戏>/<类型>.json`（开发态为 `src-tauri/resources/GameType/`） |
| 运行时读取 | `gametype::get_game_type_list()` → `bundled_gametype_folder()`，**只读这一处** |
| 用户配置目录 | 不再写入、不再读取、不再同步；`%LOCALAPPDATA%\SSMT4GlobalConfigs\GameType` 与程序无关 |
| 新增类型 | 写进 `resources/GameType/`（随构建/安装发布）；装好后直接生效，无需重启应用 |
| 升级/卸载 | 覆盖安装即整库换代；卸载即整库消失，不留残留 |

### 7.3 为什么当初会放到用户目录（回答"有什么好处"）

诚实地说，**唯一真实的好处是"给用户或 AI 一个可写落点"**：不重新构建程序也能现场补类型。随包发布的 `gametype-analysis` 技能正是这么用的——它按 MCP 拿到的"全局配置目录"生成缺失类型并写进去，让提取立刻能用。

但这个好处的代价是把「程序内容」放进了「用户数据」，于是：

- `uninstall.exe` 不会删 `%LOCALAPPDATA%`（用户数据目录，卸载器默认可留），**卸载清不干净**；
- 覆盖安装只换程序与 `resources`，**不碰用户目录**，旧类型永远留着；
- 于是出现 §3.4 的实锤：仓库在 `75f00ac` 删掉的 5 个 IdentityV 死类型，至今仍从用户目录被加载。

也就是说：把类型库放用户目录换来的"可现场追加"，被"卸载不干净 + 重装不覆盖 + 旧残留永远生效"抵消掉了；而"现场追加"这个需求，真正需要的只是一个**可写的类型库目录**，不需要它位于用户配置下。

### 7.4 本轮改动

| 文件 | 改动 |
| --- | --- |
| `src-tauri/src/gametype/mod.rs` | 全文重写：删掉 §6 的交付清单机制（`DeliveryManifest` / `deliver_*` / `write_delivered_file` / `content_hash` 及 5 个 delivery 测试）与 `runtime_gametype_folder()`；`external_game_type_folder()` 泛化为 `resolve_game_type_folder(root, game_name)`；`get_game_type_list()` 只读 `bundled_gametype_folder()`。测试改为 `resolve_game_type_folder_matches_case_insensitively`、`resolve_game_type_folder_returns_none_for_unknown_game`，保留 `all_game_type_folders_load_from_json`、`identityv_loads_user_added_split2_type` |
| `src-tauri/src/lib.rs` | 删除启动时的类型交付调用与日志（原 124-142 行整段） |
| `src-tauri/src/config/path_manager.rs` | 更新两处注释：`bundled_gametype_folder()` = 运行时唯一来源；`ssmt_gametype_folder()` = 历史版本的用户目录副本，运行时已不再读取（函数保留，已无调用者） |
| 16 个提取后端 + `common/frame_analysis/frameanalysis_base.rs`（共 18 处） | `PathManager::ssmt_gametype_folder()` → `PathManager::bundled_gametype_folder()`：`extract_new/{ailimit,apmi2,efmi3,cami,gf2,gimi,himi,identityv,naraka,narakam,ntemi,snowbreak,srmi,yysls,zzmi}.rs` 各 1 处、`zzmi_dx12.rs` 2 处、`frameanalysis_base.rs:55` |
| `src/views/ModsManagement/Mod3DPreview.vue` | 3D 预览的 GIMI 布局改为读安装目录（删掉上一轮加的用户目录优先项），与后端同源 |
| `src-tauri/resources/GameType/README.md` | 运行时位置段改为"只读装机版这一份，唯一来源"；新增类型段改为"放 `resources/GameType/`，随构建发布"；删掉交付清单段 |
| `resources/GameType/gametype-analysis/` | `SKILL.md`：类型库路径改为 `GlobalConfig.SSMTResourcesFolder()` / `<resources folder>/GameType`；`agents/openai.yaml`、`scripts/gametype_ib_check.py`、`scripts/gametype_add_missing_types.py` 的帮助文本同步；`scripts/gametype_common.py` 删掉用户目录候选根；`references/PROJECT-CONVENTIONS.md` 的运行位置与"加完类型"步骤同步 |
| `src/i18n/locales/*.json`（10 个语言） | `analyzeGameTypePrompt` 里的类型库目录来源由 `GlobalConfig.SSMT4GlobalConfigsFolder()` 改为 `GlobalConfig.SSMTResourcesFolder()` |

### 7.5 验证

- 残留符号扫描（`deliver_bundled_gametype_to_config` / `DeliveryReport` / `DeliveryManifest` / `DELIVERY_MANIFEST` / `runtime_gametype_folder` / `sync_folder` / `sync_bundled_gametype`）：`src/` 下 **0 命中**。
- `cargo test gametype`（cwd `src-tauri`，`TAURI_CONFIG='{"bundle":{"resources":[]}}'`）：**7 passed; 0 failed**，含 `gametype::tests::{resolve_game_type_folder_matches_case_insensitively, resolve_game_type_folder_returns_none_for_unknown_game, identityv_loads_user_added_split2_type, all_game_type_folders_load_from_json}`。
- `cargo check --lib`：`Finished dev profile [unoptimized] target(s) in 25.18s`，无错误、无本次改动引入的警告。
- `bunx vue-tsc --noEmit`：exit=0。
- 10 个语言文件的替换脚本逐语言断言"恰好 1 处命中 + JSON 合法 + 替换后不含 `SSMT4GlobalConfigsFolder`"，全部 ok。

### 7.6 代价与影响（需要知道的）

- **写权限**：装机版的 `resources/GameType` 在系统盘安装（如 `Program Files`）时可能需要管理员权限才能写；便携版/自定义目录无此问题。`gametype-analysis` 现在写这里，遇到权限失败应提示用户以管理员运行或改用便携版（脚本会直接报 IO 错误）。
- **旧副本变惰性残留**：`%LOCALAPPDATA%\SSMT4GlobalConfigs\GameType` 不再被读写，但对已升级的机器仍存在，可以手工删除；删掉不会影响任何功能。
- **降级**：若从新版回退到旧版程序，旧版读的是用户目录，会找不到类型库（用户目录已被清空/不再维护）。用旧版前需自行把 `resources/GameType` 复制回用户目录。
- **开发态落点**：`bun tauri dev` 下类型库路径是 `src-tauri/resources/GameType/`（`ssmt_resources_folder()` 的开发回退），技能脚本在该工作区能自行定位到它，无需配置。

（本轮改动**未提交**；按仓库 AGENTS.md，提交需用户明确要求。）

