## 6/11 脑暴笔记 — 节点化 Agent 框架

今晚的核心发现：Agent 框架里的工具天然是节点。

不是"把工具包装成节点"，而是工具本身就是节点——输入端口、输出端口、执行逻辑，一个不差。Loop 和 Model 不是节点，它们是调度器，负责决定哪些节点该跑、按什么顺序跑。这个区分很关键，很多人做节点系统的时候把调度器也当节点做了，结果图变得混乱，用户根本看不懂。

### 连线即权限

这是今晚最漂亮的一个点。Sub-agent 能用哪些工具，不是配置文件里写的白名单，不是代码里 hardcode 的列表，而是画布上的连线。你连了 file_ops 这个 cluster，sub-agent 就能读写文件；没连，模型根本不知道这些工具存在。断开一条线 = 收回一类权限。

为什么这很重要？因为现在市面上的 agent 框架（LangChain、AutoGen、CrewAI）处理 sub-agent 工具限制的方式都很丑。要么是 config 里一长串 tool_names 列表，要么是代码里 if-else 判断。用户根本搞不清 sub-agent 到底能干什么。而连线是最直觉的权限表达方式——你看得见它连了什么，就代表它能干什么。

### 工具聚类（B+C 合并）

原来有两条路：B 是静态 prompt + 运行时拒绝未连接工具，C 是把 22 个工具聚合成 6-7 个大类。单独做 B 的问题是 prompt 还是很长（22 个工具的描述全塞进去），单独做 C 的问题是聚类之后工具粒度变了不好用。合在一起做才是正解：

先聚类（22 → 7 cluster），每个 cluster 有一段固定的 sys_prompt_fragment。主 agent 的 system prompt = base_prompt + 全部 cluster fragment，稳定不变，prompt cache 命中率拉满。Sub-agent 的 system prompt = base_prompt + 已连接 cluster 的 fragment，只看到自己该看的部分。模型调了未连接的工具？运行时拒绝，返回"这个工具不在你的权限范围内"。

这样 prompt 既短又稳定。同样的工作流每次跑出来的 system prompt 完全一样 → cache 命中。

### 商业化

卖工作流。一个 JSON 文件 = 一个可复用的 agent 工作流。跟 ComfyUI 卖工作流一模一样的逻辑。用户画好节点、连好线、导出 JSON，别人导入就能用。这个市场 ComfyUI 已经验证过了，只是没人把同样的模式搬到 coding agent 领域。

### 今晚实际落地了什么

工具聚类重构已经写进代码了（commit `96f30c9`）：

- `ClusterRegistry`：7 个 cluster，每个带 sys_prompt_fragment
- `ToolRegistry` 新增 cluster 感知：allowed_clusters 过滤、execute() 权限检查、with_clusters() 创建受限副本
- System prompt 改用 cluster 描述，不再逐工具堆叠

效果：主 agent 全能力，sub-agent 一行代码限制工具范围，prompt cache 友好。

### 还没解决的问题

**可解释性 vs 黑箱。** 节点图的好处是可视化、直观，但 agent 的行为本质上还是由 LLM 驱动的。你画了一个漂亮的工作流，模型可能不按连线走（幻觉调用、跳步执行）。节点图给人"确定性"的错觉，但底下是概率性的。怎么让用户既觉得"我看懂了系统在干什么"，又不会因为模型犯错而感到被欺骗？

可能的方向：执行轨迹可视化（每一步实际走了哪条线、调了什么工具、输出了什么），而不是只展示静态的节点图。类似 ComfyUI 执行时节点会亮起来、数据流会动画化。让用户看到"实际发生了什么"而不只是"设计意图是什么"。

**UI/WebUI 怎么搞。** 两条路：寄生 ComfyUI（用 LiteGraph.js + 自己的 Rust 后端），或者自建前端。寄生路线快但受限——ComfyUI 的前端是为图像生成优化的，很多交互（agent 对话面板、工具输出展示、流式文本）得自己魔改。自建前端慢但自由度高。

还有一个更根本的问题：coding agent 的用户真的需要节点编辑器吗？写代码的人习惯命令行和配置文件，拖拽节点对他们来说可能是降级而不是升级。也许正确的产品形态不是"纯节点画布"，而是"命令行 + 可选的可视化面板"——平时用 CLI 干活，需要调试/分享/复现的时候打开 WebUI 看节点图。

**MCP 和 Skill 怎么节点化。** MCP server 提供的是动态工具集合（一个 server 可能有 1-N 个工具），Skill 是预打包的工具+prompt 组合。这两类东西怎么塞进节点图里还没想清楚。初步想法是 MCP server 启动时展开为独立节点，Skill 展开为子图。但展开之后图的复杂度会爆炸，需要折叠/展开机制。

### 下一步

Sub-agent 接上 cluster 过滤（加个 allowed_clusters 参数让主 agent 指定 sub-agent 能用什么），或者先把 LiteGraph.js 的 PoC 跑起来验证前端可行性。哪个先做取决于产品方向——先验证技术还是先验证用户体验。
