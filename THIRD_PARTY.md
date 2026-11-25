# 第三方组件

| 组件 | 版本和来源 | 许可 |
| --- | --- | --- |
| 水杉输入引擎 | [msime-engine 0.2.0](https://static.crates.io/crates/msime-engine/msime-engine-0.2.0.crate) | 引擎代码采用 MPL-2.0。内置表和改编代码的单独声明见上游归档里的 `LICENSE` 和 `licenses/`。 |
| 水杉中英文词库 | [全拼数据包 2.0.41](https://github.com/metasequoiaime/msime/releases/tag/msime-engine-capi-v0.1.2) · [词条源码](https://github.com/metasequoiaime/msime-dictionary/tree/8e8b7817c91600913625edb7995a7397fbb6899a) · [构建工具](https://github.com/metasequoiaime/msime/tree/09a69f20b4726f75d4aa318975e3e891c1135aaf/crates/dict-builder) | 主体为 GPL-3.0，另含 CC BY-SA 4.0、CC BY 4.0、MIT、WTFPL 和 SCOWL 声明。完整说明随词库下载并收入便携包。 |
| 神经联想 | [chinese-ime-lm 0.2.1](https://crates.io/crates/chinese-ime-lm/0.2.1) · [model-v1 权重](https://github.com/metasequoiaime/chinese-ime-lm/releases/tag/model-v1) | 推理库和权重采用 Apache-2.0。[模型署名](models/attribution.txt)保留 C4 和 LCCC 训练数据来源。 |
| OpenVR | [Valve OpenVR 的固定版本](https://github.com/ValveSoftware/openvr/tree/0924064316de3effbcd1acf1e309182a2deb1c05) | [BSD-3-Clause](vendor/OPENVR-LICENSE.txt)。 |

词库的二元、三元统计表含来自中文维基百科 20260901 数据转储的统计结果，署名 Wikipedia contributors，采用 CC BY-SA 4.0。

便携包的 `licenses/` 包含上游声明及 Rust 依赖的版本、许可和源码地址。
