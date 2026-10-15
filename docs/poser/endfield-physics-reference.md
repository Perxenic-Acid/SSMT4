# Endfield-Poser 衣物实现对照

源码基准：[OedoSoldier/Endfield-Poser，e7da6db](https://github.com/OedoSoldier/Endfield-Poser/tree/e7da6db6f04a86fc27d9d5bd906a70920dfb71af)。
这份对照区分已经读到的实现与 SSMT 后续方案，不把文档中的效果描述当作本项目实机验证。

## 实际采用的物理路线

[播放器说明](https://github.com/OedoSoldier/Endfield-Poser/blob/e7da6db6f04a86fc27d9d5bd906a70920dfb71af/docs/mmd-player.md)
明确使用游戏原生衣物模拟，并未实现 PMX 的 MMD 刚体 / 关节物理。
对应代码接入 BeyondDynamicBone 的原生粒子、网格、碰撞与显示任务。
它既有通用生命周期和几何处理，也有按资源身份匹配的衣物适配；不能概括成不需要模型配置的通用物理解算器。

| 源码中确认的做法 | 对 SSMT 当前问题的意义 | 适用边界 |
| --- | --- | --- |
| 固定端由动画身体骨与捕获的相对参考决定，同组件可有多个固定端 | 防止把同一衣物上一步物理结果当作下一步驱动，固定点不能任意漂移 | 原生固定粒子、动画骨与模拟骨是不同角色，须从配置和层级核验 |
| 距离约束之后再次串接原生边碰撞与归约任务 | 后续长度 / 显示约束可能重新制造穿入，统计须看最后写回 | 可借鉴约束次序，不能复用另一游戏的 job 布局或 hook 地址 |
| 稀疏骨链可重建模拟网格、添加控制点并重映射蒙皮 | 骨段胶囊没有覆盖到的裙摆三角形，不能只靠调阻尼补足 | 涉及源网格、固定区、接缝、LOD 与权重，不能只加一个碰撞函数 |
| 修改候选蒙皮前识别重叠表面并保留重复顶点组 | 多层衣物与接缝不能随意当成单张连续布处理 | 圆柱投影分区有几何前提；不是通用内外判定 |
| 检查蒙皮候选的边长与接缝分离 | 代理分离不代表衣物变形正确 | 检测或回退不是穿模求解器，也不是全动作通过证明 |

## 可复用的具体思路

### 固定点与身体驱动

[`cloth_attachment.h`](https://github.com/OedoSoldier/Endfield-Poser/blob/e7da6db6f04a86fc27d9d5bd906a70920dfb71af/src/math/cloth_attachment.h)
把根相对动画身体的平移 / 旋转记录下来，运行时从身体重建；多个固定点共同形成参考。
SSMT 的 `AnchorPose` 已明确使用动画父骨与 STATIC 局部旋转，这个原则应继续保持。
但物理子步也必须使用自洽的身体姿态：逐骨插值世界位置走的是旋转弧的弦，会缩短父子距离。
局部位置 / 旋转插值后沿 STATIC 父链重建，才能保留固定骨长；输入中明确存在的中心 / IK 局部位移也必须保留。

[`cloth_turn_motion.h`](https://github.com/OedoSoldier/Endfield-Poser/blob/e7da6db6f04a86fc27d9d5bd906a70920dfb71af/src/math/cloth_turn_motion.h)
另行处理补充惯性：同一动画姿态不重复产生冲量，跳时 / 换代 / 不连续运动重置历史，
将多次身体采样平均为加速度后交给原生积分。不能把已乘过 dt 的冲量再交给会乘 dt 的接口当力。
这部分是额外响应，不承担主体碰撞。

### 最后一次约束和最终显示

[`cloth_bonecloth_contact_finish.h`](https://github.com/OedoSoldier/Endfield-Poser/blob/e7da6db6f04a86fc27d9d5bd906a70920dfb71af/src/cloth/bonecloth/cloth_bonecloth_contact_finish.h)
核验任务身份与依赖链，在第二轮距离约束之后追加边碰撞和归约，并保持原生任务依赖。
它不是在最终 Transform 上任意推开一根骨骼。

[`cloth_display_state.h`](https://github.com/OedoSoldier/Endfield-Poser/blob/e7da6db6f04a86fc27d9d5bd906a70920dfb71af/src/cloth/collision/cloth_display_state.h)
还为其已接管的动态粒子构造私有显示输入，移除会再次限缩结果的显示根限制；固定粒子与其他团队保持原输入。
这说明即使模拟内部已经解开，输出消费者仍可能推翻结果，必须核验最终消费者。

SSMT 目前在 Bullet 积分之后还进行 FK 合回、身体接触、自接触及根位移投影。
已有分阶段夹具证明这些阶段本身仍能制造较大跳变。
下一项约束调整应同时观察最后几何与连续性，不能仅增加投影迭代、扩大根自由度，或只测试候选终点。

### 模拟表面和蒙皮

[`cloth_asset_surface.h`](https://github.com/OedoSoldier/Endfield-Poser/blob/e7da6db6f04a86fc27d9d5bd906a70920dfb71af/src/cloth/assets/cloth_asset_surface.h)
从实际蒙皮权重和三角形挑选表面，用空间分区识别重叠区域，保留不适合处理的区域与其邻面。
同位置、同权重的重复顶点成组处理，避免同一接缝两边采用不同规则。

[`cloth_skin_guard.h`](https://github.com/OedoSoldier/Endfield-Poser/blob/e7da6db6f04a86fc27d9d5bd906a70920dfb71af/src/cloth/core/cloth_skin_guard.h)
对原始 / 候选权重计算最终蒙皮边长与接缝距离。代码的恢复条件针对无效输入或接缝分离；
不能把所有拉伸标记都误读为自动回退，更不能称为完整碰撞保护。

[`cloth_asset_body_contact.h`](https://github.com/OedoSoldier/Endfield-Poser/blob/e7da6db6f04a86fc27d9d5bd906a70920dfb71af/src/cloth/assets/cloth_asset_body_contact.h)
展示按高身体权重顶点的横截面拟合接触体的例子。
该路径有明确的源资源与网格身份约束，横截面方向和数量不能直接当作所有角色的通用常数。

## 对现有实现的取舍

- 保持 STATIC 骨架、同角色 inverse bind 与真实顶点权重为参考，不换用运行中的变形结果重建静态配置。
- 不将 Endfield 的 BeyondDynamicBone 类型、任务结构和地址用于 SSMT 当前游戏。现有原生采集尚不等于可控的原生求解接口。
- 实时 / 预烘焙仍须共用同一已验证的求解语义，旧缓存依赖物理版本与配置指纹失效。
- 为真实表面补充碰撞表达时，先用已有离线网格证明约束图、固定区、接缝和最终蒙皮可行，再考虑运行时接入。
- SSMT 已增加 `SkinSurface.h` 的源边、混合固定权重及 FK 接触梯度，当前用于离线核验；尚未接入生产碰撞求解，不将此基础数据层视为完成表面物理。
- Endfield 的源码按 AGPL-3.0 发布，其衣物实现也记录了 EIEM 来源；本次仅作源码对照，未复制代码或引入其资源。

完整动作的指标和实验取舍写入 Native 调查报告；本说明不保存本机会话路径和临时测试编号。
