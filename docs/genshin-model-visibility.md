# 原神镜头淡出补充

Player Tweaks 0.1.1 沿用“关闭角色镜头淡出”开关，补充角色界面等展示场景的
镜头距离淡出处理。旧设置 `DisableCharacterFade` 的格式、默认值与启用条件不变。
不需要开启自定义 FOV、缩放解锁、DLSS5 或 Poser。

## 原因与范围

已核对原神 7.1 CN/Bilibili 本机样本：

- `YuanShen.exe` SHA-256：`7F89938DA606C1281659607D464702CDDB9A09A7D4320A196630F60A811EC38E`。
- `global-metadata.dat` SHA-256：`469ECCD43AA48FE7A2DF4E1CAF66FA1268A17F5A4A03FA209E1427E467CB0161`。
- 原有 Hook 定位到 `PlayerPerspective` 对应透明度 setter。
- `MonoSceneUISetup` 的另一条调用链根据镜头到模型骨骼的距离计算透明度，直接写入
  渲染组件，没有经过该 setter；这解释了原有反虚化无法覆盖该路径。

新增 Hook 位于这条镜头专用路径的原生透明度提交块，在应用渲染状态前改为不透明。
保留原生脏标记、应用调用、返回值及未进入计算时的 `-1.0f` 返回。
不强制开启 GameObject、Renderer 或任意隐藏理由，不改切换角色和退场的通用路径。
生产代码通过唯一特征和局部布局校验定位，不用样本 RVA 作后备地址。

这仍需实机确认是否覆盖用户遇到的全部低角度消失：近裁剪面、独立 Shader 裁剪和
其他版本新增路径不由此补丁修改。软件回归通过不能替代画面验收。

## 本地测试与发布

在开发版插件市场刷新，选择本地构建的 Player Tweaks **0.1.1**，检查来源并更新；
保留原神启用状态和镜头设置后，退出旧游戏进程，再通过 SSMT 启动。
受控开发包位于 `build/plugin-packages/ssmt.player-tweaks-0.1.1.ssmtpkg`。
正式版从后续 Native Release 的插件目录发现新版本；本地构建不等于已经发布到 GitHub。
历史 0.1.0 清单保留，但不再用当前 DLL 重新发布。

日志位置：`%LOCALAPPDATA%/SSMT4CachedFolder/Logs/SSMT-Player-Tweaks.log`。
确认本次启动出现以下两行，而不是拿旧游戏的结果判断更新：

```text
Camera.CharacterFade enabled.
Camera.SceneCharacterFade enabled (native camera opacity only).
```

如新增路径显示 disabled，保留 `Camera.SceneCameraFade candidates=... validation=...`
及禁用原因，重新核对客户端版本和热修复；不得通过去掉校验强行安装。

实机验收顺序：

1. 原神角色界面：拉近镜头、降低观察角度，再拉远，观察模型、贴图和特效是否正常。
2. 在同一界面切换多个角色，再切换属性、武器和外观页，确认旧模型正常退场，没有叠人。
3. 返回世界，检查近镜头、瞄准、战斗切人、传送和重新进入角色界面。
4. 关闭“关闭角色镜头淡出”，保存并重启游戏，确认原生淡出恢复。
5. 停用 Player Tweaks 后启动，确认普通 Mod、其他插件及正常启动保持可用。

离线验证已包括新增 Hook 的实际 SSE 提交、开关关闭、移除恢复、安装冲突、版本布局和
IFix 拒绝路径，以及原有镜头/缩放回归。测试只读映射完整样本，未启动游戏或执行完整游戏函数。
