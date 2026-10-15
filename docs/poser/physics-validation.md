# Poser 骨骼与衣物物理验证

适用于 `native/SSMT-Poser` 的动作重定向、只读源网格采集、衣物穿模和物理跳变。
源码与构建属于 native 子仓库；本文、项目 skill 和通用导出工具属于主仓库。

## 参考与证据

STATIC Avatar 定义骨架层级和局部位置，Mesh inverse bind 补充蒙皮参考。
LIVE pose 只用于恢复或有时间标识的观察，不能代替静态 A pose。
绑定时快照不等于当前实时状态；缓存姿态检查不等于当前驱动重新播放。

| 检查 | 能说明什么 | 不能代替什么 |
| --- | --- | --- |
| 骨长、STATIC 复现误差 | 层级与 inverse bind 可配对 | 动作正确、碰撞正确 |
| 胶囊 / 刚体接触 | 代理形状之间的残留 | 实际衣物表面 |
| 局部 TRS 重建 | 缓存与最终 FK 位置一致 | 碰撞形状覆盖足够 |
| 源顶点四权重蒙皮 | 实际三角形相交与变形 | 采样之间连续无穿越 |
| 相邻样本旋转 / 位移 | 跳变是否存在 | 仅靠限速取得正确接触 |
| 实机观察 | 最终渲染、光标及写入时序 | 离线重放的可重复证据 |

已有案例中，胶囊穿入小于 2 mm 时真实裙摆仍大量相交；代理验收通过不能据此标记问题解决。
STATIC 相交记录须精确到衣物 / 身体三角形对。共享顶点不意味着整片三角形永远互斥。
网格可能有开口，最近法线符号不能直接当作闭合身体的内外判定。

## 整理已有采集

`same_actor_bindposes.json` 和只读配置 JSON 必须来自同一所选 Animator / Body。
新参考包含 Mesh identity，旧参考只有在采集明确记录了外部交叉核验时才能配对。
导出工具不会自行核验游戏二进制或补足缺失布局证据。

```powershell
python scripts/poser/export_static_fixtures.py `
  --reference '<报告目录>/same_actor_bindposes.json' `
  --capture '<只读采集.json>' `
  --output-prefix 'tmp/poser-probe/fixtures/sample'
```

产出 STATIC Avatar、Body bind、SSMTSKN3 顶点 / 三角形 / 法线夹具及输入输出 SHA-256 清单。
工具只读文件，允许相同内容重复导出，拒绝覆盖不同内容；会核验父链、有限值、权重、索引和子网格范围。
不提供 capture 时可先导出 Avatar / Body。通道格式不受支持或数据不完整时应补证据，不猜测字段。
所有调查夹具、捕获数据和一次性反汇编脚本保留在忽略的 `tmp` 下。

导出器回归：`python -m unittest discover -s scripts/poser/tests -v`。
覆盖新旧身份信息、16 / 32 位索引一致性、损坏权重、索引越界、缓冲截断及 STATIC 父链异常。

## 构建与重放

使用现有 `native/build-poser` 配置，仅构建相关 targets，不调用主项目发行构建脚本。
构建失败必须先处理；不要继续执行上次遗留的测试 EXE。
同一测试 EXE 尚在运行时无法覆盖链接，等待该测试结束或终止已明确识别的本轮测试进程后再构建。

```powershell
cmake --build native/build-poser --config Release --target PoserPhysicsTests --parallel 4
if ($LASTEXITCODE -ne 0) { throw 'PoserPhysicsTests 构建失败' }
$exe = '.\native\build-poser\Release\PoserPhysicsTests.exe'
& $exe --skin-default-fixture '<Avatar>' '<Body>' '<动作.vmd>' '<Skin>'
& $exe --skin-fixture '<Avatar>' '<Body>' '<动作.vmd>' '<原生参与范围>' '<Skin>'
```

`--skin-driver-fixture` 观察不含自有物理的身体驱动与继承变形。
`--skin-pose-fixture Avatar Body Motion NativeSelectionOrDash Skin Pose` 检查已存姿态。
后者不重新播放当前驱动，不能用于当前版本的实时 / 烘焙一致性声明。
身体 / 表面分别按现有夹具采样率检查，旋转和位移连续性逐个 120 Hz 样本检查。

`--skin-window-fixture Avatar Body Motion Skin Seconds` 可缩短调查窗口；日志明确标记
`full_motion_acceptance=0`，窗口统计只能与相同起止时间的基线比较。
短窗口表现较好后仍须完成上述两种完整动作，不将短窗口峰值与完整动作峰值直接相比。

### 实际蒙皮约束数据

`SkinSurface.h` 从 STATIC inverse bind 局部坐标、四权重和源三角形建立值类型的表面模型。
当前接入离线夹具，日志明确输出 `solver=NOT_CONNECTED`；生产 Bullet 尚未使用该模型，
建图或梯度回归通过不能描述成穿模已修复。
`--skin-topology-fixture Avatar Body Motion Skin [NativeSelection]` 只核验源表面与建图，
不进行动作重放；输出 `TOPOLOGY_ONLY full_motion_acceptance=0`。

- 每个顶点保留所有有效权重和相应骨骼局部坐标。部分固定、部分模拟的顶点保留混合权重，不按最大权重整块指派。
- 身体面排除已识别但未选中的衣物候选，避免把未模拟的裙摆当作身体碰撞体。
- 身体 / 衣物分类仍依赖输入参与范围，不代表已经取得所有角色的资源材料分区。
- 边来自实际参与模拟的三角形，去掉重复的同索引边，但保留固定边界及零长度源边；不通过空间近邻凭空连接布片。
- 同位置、同蒙皮的重复顶点记录为候选组，仍保留原始索引、三角形和边。候选关系不能自动用作跨衣层缝合约束。
- 建图指纹包含参考配置、规范化影响项、参与范围和源拓扑，不散列结构体填充字节。失败建图不替换原模型。

`ContactGradient` 对世界空间关节小角度旋转求导：累加受该关节影响的每一项加权蒙皮杠杆，
再与接触法线叉乘。包含所有 FK 后代；固定骨权重不产生可移动自由度。
实际边长的梯度使用两个端点的接触梯度之差，方向取当前边的单位向量。
调用前校验配置身份和姿态；逐顶点计算不分配内存。
`Validation/SkinSurfaceTests.cpp` 用独立子树旋转的中心差分核验接触和边长梯度，
覆盖世界放置、后代、混合固定权重、接缝候选、拓扑指纹及无效输入。

`SkinStrainFixture` 另外记录最终蒙皮的边长比例、最坏源顶点对和时刻。
它排除短于 0.5 mm 的边作为相对应变分母；这不改变表面模型或三角形相交统计。
统计覆盖所选物理表面，可能包含头发、饰物与衣物；判断最坏边属于哪个部件时核对其源权重。

`SSMT_POSER_TRACE_PHYSICS` 只在 Physics 测试目标编译，分离积分与 FK、身体接触、自碰撞、根位移投影的变化。
另记单次子树旋转峰值，区分单个限位修正跳变与多个接触反复累加。
`--profile-fixture Avatar Body Motion` 只检查初始化后的参与范围及 STATIC 代理自相交，不生成烘焙。
对照阶段峰值的骨名和采样时刻，再决定修正约束、几何或写回，不通过减小碰撞形状、删除相交对来取得假通过。
普通测试与完整动作检查分别记录退出码；完整动作失败时仍报告失败，不能只引用 CTest 汇总。

## 持续维护

同步 Native 历史时，重复引入的早期提交可能产生大量 add/add 冲突。
先比较对应历史提交的实际文件树，确认远端独有内容，再决定每个冲突的保留版本。
远端补回原先被忽略的旧测试时，核对断言采用的坐标约定：历史 Basis A/B 保留 `S=I`，
当前 FK 使用已确认的 `S=Ry(pi)`。更新旧测试预期，不能为满足旧断言回退已验证的播放语义。

生产代码引用静态数据须在初始化阶段完成身份核验与有界分配，高频回调不做文件 IO 或重复发现。
更改驱动采样语义更新 `physics::MotionDriverVersion`；更改物理语义更新 `physics::Version`。
重新记录最坏姿态、连续性、真实相交和耗时，并核对缓存失效。
仅将通过验收的代码用于交付入口；失败实验不加入第三方库或正常运行时。
第三方源码提交与功能修改分开管理，主仓库 pointer 与 native 源码差异分开说明。

## 成熟实现的适用边界

Endfield-Poser 的原生任务接入、表面重建及蒙皮保护见 [源码对照](endfield-physics-reference.md)。
研究参考实现时先确认它使用原生物理、模型刚体还是自建表面；效果不能脱离对应的模型配置和输出消费者移植。
逐骨世界位置插值可能缩短转身中的骨段。子步参考应检查局部 TRS 与父链一致性，并保留明确的动画局部平移。

[Saba 的 MMD 物理](https://github.com/benikabocha/saba/blob/master/src/Saba/Model/MMD/MMDPhysics.cpp)
使用模型提供的刚体、关节、偏移和碰撞组；VMD 动作本身不能提供这些配置。
[KawaiiPhysics 的骨链求解](https://github.com/pafuhana1213/KawaiiPhysics/blob/master/Plugins/KawaiiPhysics/Source/KawaiiPhysics/Private/AnimNode_KawaiiPhysicsSimulation.cpp)
在子步中插值动画参考，处理接触后再恢复角限制及骨长，并将结果用于后续步骤。
这些流程可作实现对照，不能把其中某一个碰撞函数移植后就视为整个蒙皮系统已验证。

手动改变骨骼子树时，须检查整棵受影响子树与其他接触对象的关系。
只看当前碰撞对或修正终点，会遗漏新穿入及移动途中的穿越。
但单独禁止穿越也可能把已经穿入的骨链锁在体内；必须用完整动作同时检查残留、连续性和耗时。
胶囊绕长轴转动不改变形状占据，却会改变运动学速度和接触摩擦。
从 STATIC 局部轴继承动画骨骼参考标架，再对齐端点；单独从世界轴重建四元数会丢失这一自由度。
