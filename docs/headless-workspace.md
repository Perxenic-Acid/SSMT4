# 无界面工作空间提取

在源码根目录运行：

```powershell
cargo run --locked --manifest-path src-tauri/Cargo.toml --bin ssmt4-workspace -- request.json
```

命令读已有的帧分析目录，无需启动 SSMT4 桌面程序。请给测试任务使用独立工作空间路径。每个 LOD 选择自己的帧目录，并指定 `draw_ibs` 列表或选择 `"full_filter": "all"` 执行全量提取。命令失败时返回非零退出码。

```json
{
  "workspace": "D:/Temp/SSMT4-Test",
  "game": "ZZMI",
  "lods": [
    {
      "name": "LOD0",
      "frame": "D:/captures/FrameAnalysis-001",
      "draw_ibs": [{"hash": "f22a9f08", "alias": "发饰"}],
      "textures": [
        {
          "submesh": "f22a9f08-858-0",
          "name": "DiffuseMap",
          "hash": "c8f551a1",
          "slot": "ps-t3",
          "mark_type": "Slot"
        }
      ]
    }
  ]
}
```

`textures` 可省略。标记默认按哈希和槽位定位贴图。匹配出多个不同文件时，可在请求中填写提取结果索引中的 capture_name，写入各数据类型目录的子网格 JSON。支持 `Hash`、`Slot`、`SharedSlot` 绑定类型，以及可选的 `face_sdf_channel`。单独运行生成的可执行文件时，从副本源码根目录运行，或将 SSMT4 的 `resources` 放到该文件旁。
