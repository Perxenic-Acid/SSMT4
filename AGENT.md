# AGENT.md

本文件记录自动化开发 agent 在 SSMT 项目中工作时需要长期知晓的信息。
它用于补充 README 与源码本身，不用于记录临时任务。

## 1. 项目概述

1. 本项目名为 **SSMT**。
2. 主项目地址：
   `https://github.com/Perxenic-Acid/SSMT4.git`
3. 主项目根目录为：
   `.\`
4. SSMT 主要用于基于 3DMigoto 的游戏 Mod 管理与开发，并提供工作空间管理、
   Mod 制作辅助、运行时管理、插件系统等功能。
5. 当前主要目标平台为 Windows。

## 2. 仓库与目录关系

除主仓库外, 可以认为均为位于本机其他路径的位于根目录下的目录链接.

### 2.1 主仓库

- `.\`
- Repository: `https://github.com/Perxenic-Acid/SSMT4.git`

这是 SSMT 主应用所在仓库。

修改这里的文件属于 SSMT 主仓库本身。

### 2.2 Git Submodules

本项目目前包含两个 Git submodule。

#### 3DMigoto Runtime

- 路径：`.\runtime\`
- Repository:
  `https://github.com/Perxenic-Acid/SSMT-3DMigoto-Runtime.git`
- Upstream:
  `https://github.com/SpectrumQT/XXMI-Libs-Package.git`

该项目是 SSMT 使用的 3DMigoto Runtime 二次开发版本，负责实际的
3DMigoto/D3D11 Runtime、注入相关运行时逻辑以及与 SSMT Native 的桥接。

对 `.\runtime\` 中源码的修改属于独立 Git 仓库。
完成修改后需要在该 submodule 中单独提交，再由主仓库更新 submodule pointer。

#### SSMT Native

- 路径：`.\native\`
- Repository:
  `https://github.com/Perxenic-Acid/SSMT-Native.git`

该项目包含 SSMT 的 Native 工具与基础设施，包括但不限于：

- 游戏进程启动与 DLL 注入；
- PluginHost；
- Plugin ABI；
- Native Plugins；
- 游戏运行时工具；
- 与 3DMigoto Runtime 的 Native bridge。

对 `.\native\` 中源码的修改同样属于独立 Git 仓库。

不要把 submodule 内的修改误认为主仓库修改。

## 3. 关联项目

以下目录存在于统一开发工作区中，但不是 SSMT 主仓库的 submodule。

### TheHerta

- 路径：`.\BlenderPlugin\`
- Repository:
  `https://github.com/Perxenic-Acid/TheHerta4.git`

TheHerta 是 Blender 插件，用于 Blender 端的 Mod 制作、模型与材质处理等功能。

除非任务明确涉及 Blender 侧功能，否则不要修改该项目。

### SSMT Documents

- 路径：`.\WebDocuments\`
- Repository:
  `https://github.com/Perxenic-Acid/SSMT4-Documents.git`

基于 VitePress 的 SSMT 文档项目。

除非任务明确要求同步或修改文档，否则代码修改不应自动扩散到该项目。

## 4. 工作空间元信息存储库

当前工作空间元信息存储库：

`https://github.com/Perxenic-Acid/SSMT-WorkSpace_Access-Library`

该仓库用于存储 SSMT 工作空间相关元信息。

不要把它视为 SSMT 源码仓库，也不要在没有明确任务要求时修改其内容。

## 5. 技术栈

SSMT 是多语言项目。修改代码前应首先确认当前所在模块。

主应用主要涉及：

- Tauri
- Rust
- Vue
- TypeScript / JavaScript
- Vite

Native 项目主要涉及：

- Rust
- C++
- Windows API
- D3D11 / DXGI
- DLL injection
- C ABI / FFI

3DMigoto Runtime 主要涉及：

- C++
- D3D11 / DXGI
- 3DMigoto
- Windows API

TheHerta 主要涉及：

- Python
- Blender Python API (`bpy`)

文档项目主要涉及：

- VitePress
- Markdown
- Vue

不要为了统一代码风格而跨语言、跨仓库进行无关重构。

## 6. Git 与 Submodule 工作规则

开始较大任务前，应先检查：

    git status
    git submodule status

如果任务涉及 `runtime` 或 `native`，还应进入对应 submodule 检查：

    git status
    git log -1 --oneline

修改 submodule 时：

1. 在 submodule 内完成源码修改；
2. 在 submodule 内构建/测试；
3. 不要擅自丢弃已有未提交修改；
4. 不要使用 destructive Git command 清理用户工作；
5. 修改完成后明确区分：
   - submodule 自身的源码 diff；
   - 主仓库中的 submodule pointer 变化。

除非任务明确要求，否则不要自动 commit、push、rebase、reset 或 force push。

## 7. 修改原则

### 7.1 先理解现有实现

修改前优先：

1. 阅读相关源码；
2. 搜索现有实现和调用点；
3. 检查相邻模块是否已有相同基础设施；
4. 确认实际调用链；
5. 再进行修改。

不要仅根据文件名或局部代码猜测项目架构。

### 7.2 最小必要修改

默认采用满足任务要求的最小修改范围。

不要顺手：

- 大规模重命名；
- 格式化无关文件；
- 重写已有模块；
- 改变无关公开 API；
- 添加没有实际需求的抽象层；
- 因个人偏好替换现有技术方案。

如果发现值得重构但与当前任务无关的问题，应指出，而不是自动扩大修改范围。

### 7.3 保持现有架构决策

项目中已经存在明确设计的基础设施时，应优先扩展现有设计，而不是创建平行实现。

尤其注意：

- Plugin ABI 的兼容性；
- Native / Runtime 的职责边界；
- 3DMigoto 上游代码与 SSMT 自定义代码的边界；
- 高频渲染路径的性能；
- Windows DLL / COM / FFI 生命周期。

## 8. Native / Plugin ABI 规则

涉及 Plugin ABI 时：

1. FFI struct 使用稳定 C layout，例如 `#[repr(C)]`；
2. ABI 结构原则上采用 append-only 扩展；
3. `struct_size` 表示 caller 实际提供的结构容量；
4. receiver 不得访问 `struct_size` 以外的字段；
5. 新增尾字段必须进行 size gating；
6. ABI family 内仅尾部兼容扩展时，不应无理由提升 ABI version；
7. breaking layout/semantic change 才考虑提升 ABI version；
8. Rust panic 不得跨越 C ABI/FFI boundary；
9. COM pointer 的 ownership 必须明确区分 borrowed 与 owned。

修改 ABI 后，应同步检查现有 ABI compatibility tests。

## 9. 高频运行时路径

涉及 Present、Draw、渲染事件或其他高频 callback 时，默认认为它们属于性能敏感路径。

正常热路径应尽量避免：

- heap allocation；
- Mutex；
- 文件 IO；
- 每帧日志；
- 每帧字符串格式化；
- 每帧 `GetProcAddress`；
- 每帧重复 discovery；
- 不必要的 metadata lookup。

初始化阶段可以完成的工作，不应推迟到每帧执行。

## 10. 3DMigoto Runtime 修改原则

`.\runtime\` 是基于上游项目进行的二次开发。

修改时应尽量：

- 保持 SSMT 自定义逻辑局部化；
- 避免无必要的大规模修改上游代码；
- 尽可能通过明确的 bridge / hook point 接入；
- 降低未来同步 upstream 时的冲突成本。

不要为了 SSMT 功能重写与该功能无关的 3DMigoto 上游实现。

## 11. 游戏相关 Native 代码

游戏相关的 Pattern、Hook、Patch 等逻辑具有版本依赖性。

不要假设：

- 地址长期稳定；
- Pattern 长期稳定；
- 不同游戏或不同版本具有相同内部结构。

Pattern Scanner 找到的结果必须进行必要的数量或有效性检查。

涉及游戏二进制行为时，如果现有证据不足，应明确指出需要实际游戏、
反汇编、日志或 Frame Analysis 验证，而不是凭猜测硬编码。

## 12. 构建与验证

修改完成后，应尽可能执行与修改范围对应的最小充分验证。

例如：

- Rust 修改：`cargo check` / `cargo build` / 相关 tests；
- C/C++ 修改：使用项目已有 CMake 配置构建相关 target；
- Vue/TypeScript 修改：执行项目已有 typecheck/build；
- ABI 修改：执行 ABI compatibility tests；
- Runtime 修改：至少完成对应架构的编译验证。

不要因为整个大型项目存在与当前任务无关的环境问题，就跳过能够独立执行的局部验证。

如果无法执行某项验证，应明确说明：

- 未执行什么；
- 为什么无法执行；
- 哪些风险因此仍未验证。

## 13. 依赖与生成文件

不要手工修改：

- build 输出；
- `target`；
- `node_modules`；
- CMake generated files；
- 编译生成的 DLL/EXE；
- 其他明确属于生成物或缓存的文件。

除非任务本身就是处理这些文件。

引入新依赖前，应先判断现有依赖或标准库是否已经能够完成任务。

不要为了很小的功能增加重量级依赖。

## 14. 文档与注释

注释应解释：

- 非显然的设计原因；
- ABI / ownership / lifetime；
- 与上游行为不同的 SSMT 特有逻辑；
- 容易被未来维护者错误“简化”的约束。

不要给显而易见的代码逐行添加解释性注释。

如果修改改变了用户可见行为、公开接口或项目架构，应检查 README / WebDocuments
是否需要同步，但不要在没有必要时制造文档改动。

## 15. Agent 工作约定

遇到以下情况时，不要自行猜测：

- 需求存在多个会显著改变架构的解释；
- 修改可能破坏 ABI；
- 需要决定是否修改 upstream-derived code；
- 需要删除或迁移大量已有代码；
- 发现用户工作区存在与任务冲突的未提交修改；
- 缺少只能通过真实游戏运行获得的信息。

此时应先说明发现的问题以及可选方案。

对于能够从源码、Git 历史、测试或已有项目结构中确定的问题，
应自行调查后继续，不要把可以自行解决的问题反问给用户。