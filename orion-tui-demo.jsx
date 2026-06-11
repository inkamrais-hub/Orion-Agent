import { useState, useEffect, useRef } from "react";
import {
  MessageSquare, Cpu, Layers, Wrench, Database, Shield, Settings,
  Play, Send, ChevronRight, Terminal, FileText, Search, Globe,
  GitBranch, Zap, Eye, Lock, BarChart3, Activity, Box, Users,
  ArrowRight, Check, X, Clock, AlertTriangle, Hash, Code,
  FolderTree, MessageCircle, Bot, Workflow, Gauge, Server,
  ChevronDown, ChevronUp, Filter, RefreshCw, Star, Radio,
  Circle, Sparkles, BookOpen, Target, TrendingUp
} from "lucide-react";
import {
  AreaChart, Area, XAxis, YAxis, Tooltip, ResponsiveContainer,
  BarChart, Bar, PieChart, Pie, Cell, RadialBarChart, RadialBar
} from "recharts";

// ============================================================
// Orion Agent Framework — TUI Demo
// 猎户座 Agent 框架前端演示
// ============================================================

const COLORS = {
  bg: "#0a0e1a",
  surface: "#111827",
  surfaceHover: "#1a2236",
  border: "#1e293b",
  borderLight: "#334155",
  primary: "#6366f1",
  primaryLight: "#818cf8",
  accent: "#f59e0b",
  accentGreen: "#10b981",
  accentRed: "#ef4444",
  accentCyan: "#06b6d4",
  accentPurple: "#a78bfa",
  textPrimary: "#f1f5f9",
  textSecondary: "#94a3b8",
  textMuted: "#64748b",
};

// ---- Mock Data ----
const tokenUsageData = [
  { t: "00:00", input: 1200, output: 800 },
  { t: "00:05", input: 2400, output: 1600 },
  { t: "00:10", input: 1800, output: 2200 },
  { t: "00:15", input: 3200, output: 2800 },
  { t: "00:20", input: 2600, output: 3400 },
  { t: "00:25", input: 4100, output: 3000 },
  { t: "00:30", input: 3600, output: 4200 },
];

const toolDistData = [
  { name: "bash", value: 32, color: "#6366f1" },
  { name: "read", value: 28, color: "#06b6d4" },
  { name: "write", value: 18, color: "#f59e0b" },
  { name: "edit", value: 14, color: "#10b981" },
  { name: "grep", value: 12, color: "#a78bfa" },
  { name: "glob", value: 10, color: "#ec4899" },
  { name: "others", value: 8, color: "#64748b" },
];

const cacheHitData = [
  { name: "L1命中", value: 68, fill: "#6366f1" },
  { name: "L2命中", value: 22, fill: "#06b6d4" },
  { name: "未命中", value: 10, fill: "#334155" },
];

const auditEvents = [
  { id: 1, type: "SessionStart", time: "14:32:01", detail: "session_0x7f3a 启动, model: deepseek-chat", level: "info" },
  { id: 2, type: "LlmRequest", time: "14:32:03", detail: "input: 2,340 tokens → output: 856 tokens", level: "info" },
  { id: 3, type: "ToolCall", time: "14:32:05", detail: "bash: cargo build --release (exit: 0, 3.2s)", level: "info" },
  { id: 4, type: "ToolCall", time: "14:32:08", detail: "read: src/core/loop.rs (4,521 bytes)", level: "info" },
  { id: 5, type: "ToolCall", time: "14:32:09", detail: "write: src/tools/new_tool.rs (1,203 bytes)", level: "info" },
  { id: 6, type: "SecurityEvent", time: "14:32:12", detail: "bash 风险命令拦截: rm -rf / (denied by guardrail)", level: "warn" },
  { id: 7, type: "ToolCall", time: "14:32:15", detail: "grep: 'fn run_simple_loop' → 3 matches", level: "info" },
  { id: 8, type: "LlmRequest", time: "14:32:18", detail: "input: 4,120 tokens → output: 1,230 tokens", level: "info" },
  { id: 9, type: "FileOperation", time: "14:32:20", detail: "snapshot: src/core/loop.rs → .orion/snapshots/", level: "info" },
  { id: 10, type: "Error", time: "14:32:22", detail: "provider timeout: deepseek-chat (30s → retry)", level: "error" },
  { id: 11, type: "ToolCall", time: "14:32:25", detail: "symbol_search: 'Provider' trait → found in core/provider.rs", level: "info" },
  { id: 12, type: "ConfigChange", time: "14:32:28", detail: "orchestrator.mode: sequential → parallel", level: "info" },
];

const tools = [
  { name: "read", desc: "读取文件内容", category: "核心", risk: "Safe", icon: "FileText", count: 28 },
  { name: "write", desc: "创建/覆盖文件", category: "核心", risk: "Medium", icon: "FileText", count: 18 },
  { name: "edit", desc: "精确字符串替换编辑", category: "核心", risk: "Low", icon: "Code", count: 14 },
  { name: "bash", desc: "执行 Shell 命令 (5级风险)", category: "核心", risk: "Safe→Critical", icon: "Terminal", count: 32 },
  { name: "glob", desc: "文件名模式匹配搜索", category: "搜索", risk: "Safe", icon: "Search", count: 10 },
  { name: "grep", desc: "正则内容搜索", category: "搜索", risk: "Safe", icon: "Search", count: 12 },
  { name: "symbol_search", desc: "代码符号搜索 (AST)", category: "代码智能", risk: "Safe", icon: "Code", count: 6 },
  { name: "find_callers", desc: "函数调用链分析", category: "代码智能", risk: "Safe", icon: "GitBranch", count: 4 },
  { name: "project_map", desc: "项目结构概览", category: "代码智能", risk: "Safe", icon: "FolderTree", count: 3 },
  { name: "web_search", desc: "Web 搜索引擎", category: "外部", risk: "Safe", icon: "Globe", count: 5 },
  { name: "create_sub_agent", desc: "创建子 Agent", category: "编排", risk: "Low", icon: "Bot", count: 8 },
  { name: "send_message", desc: "Agent 间消息传递 (A2A)", category: "编排", risk: "Low", icon: "MessageCircle", count: 6 },
  { name: "list_peers", desc: "列出可通信 Agent", category: "编排", risk: "Safe", icon: "Users", count: 2 },
  { name: "ask_user", desc: "向用户提问", category: "交互", risk: "Safe", icon: "MessageSquare", count: 3 },
  { name: "mcp_call", desc: "MCP 协议工具调用", category: "外部", risk: "Low", icon: "Box", count: 7 },
  { name: "multi_shell", desc: "多终端并行管理", category: "核心", risk: "Medium", icon: "Terminal", count: 4 },
  { name: "web_fetch", desc: "网页内容抓取", category: "外部", risk: "Safe", icon: "Globe", count: 3 },
];

const chatMessages = [
  { role: "user", content: "帮我分析一下 src/core/loop.rs 的核心执行循环，并找出所有使用 StepObserver 的地方。" },
  { role: "assistant", content: "我来分析核心执行循环并搜索 StepObserver 的使用位置。", thinking: "用户想了解 loop.rs 的核心逻辑和 StepObserver 的集成点。我需要先读取文件内容，然后搜索 StepObserver 的所有引用。这两个操作可以并行执行。", tools: [
    { name: "read", input: '{ path: "src/core/loop.rs" }', status: "done", result: "读取成功 (4,521 bytes, 187 行)" },
    { name: "grep", input: '{ pattern: "StepObserver", path: "src/" }', status: "done", result: "找到 5 处匹配" },
  ]},
  { role: "assistant", content: "## 分析结果\n\n`run_simple_loop()` 是 Agent 的核心心跳循环，实现了 **思考 → 工具调用 → 观察** 的迭代模式：\n\n**核心流程：**\n1. `Provider.stream()` — 流式调用 LLM 获取响应\n2. 解析 `ContentBlock::ToolUse` — 提取工具调用请求\n3. `ToolRegistry.execute()` — 执行工具（只读工具自动并行）\n4. `StepObserver.observe()` — 观察执行结果并决策\n\n**StepObserver 的 4 种决策：**\n- `Continue` — 正常继续下一轮\n- `Retry` — 工具失败，重试（最多 2 次）\n- `Replan` — 当前方案不通，注入提示让 LLM 换思路\n- `EarlyStop` — 已获得足够信息，提前终止\n\nStepObserver 在以下 5 个位置被使用：\n- `loop.rs:42` — 初始化\n- `loop.rs:87` — 工具执行后观察\n- `loop.rs:112` — 错误分类判断\n- `loop.rs:134` — 重试决策\n- `loop.rs:156` — 上下文压缩触发点" },
  { role: "user", content: "创建一个子 Agent 来审查这段代码的安全性。" },
  { role: "assistant", content: "好的，我来创建一个 Review 类型的子 Agent 进行安全审查。", tools: [
    { name: "create_sub_agent", input: '{ type: "review", task: "审查 src/core/loop.rs 的安全性，关注权限绕过、注入风险、资源泄漏等" }', status: "running", result: null },
  ]},
];

const orchestrationDAG = {
  coordinator: { name: "Coordinator", model: "deepseek-chat", status: "active" },
  tasks: [
    { id: "T1", name: "需求分析", status: "done", worker: "Worker-Search", deps: [], duration: "12s" },
    { id: "T2", name: "代码生成", status: "done", worker: "Worker-Code", deps: ["T1"], duration: "45s" },
    { id: "T3", name: "单元测试", status: "running", worker: "Worker-Code", deps: ["T2"], duration: "28s" },
    { id: "T4", name: "安全审查", status: "pending", worker: "Worker-Review", deps: ["T2"], duration: "-" },
    { id: "T5", name: "集成验证", status: "pending", worker: "Worker-Code", deps: ["T3", "T4"], duration: "-" },
  ],
};

const sessions = [
  { id: "sess_0x7f3a", model: "deepseek-chat", created: "2026-06-10 14:32", turns: 12, tokens: 48200, status: "active", size: "24.3 KB" },
  { id: "sess_0x6e2b", model: "claude-sonnet-4", created: "2026-06-10 11:15", turns: 28, tokens: 124500, status: "idle", size: "89.7 KB" },
  { id: "sess_0x5d1c", model: "qwen-max", created: "2026-06-09 16:40", turns: 8, tokens: 32100, status: "idle", size: "15.2 KB" },
  { id: "sess_0x4c0d", model: "deepseek-chat", created: "2026-06-09 09:20", turns: 45, tokens: 256000, status: "archived", size: "156 KB" },
  { id: "sess_0x3b9e", model: "gpt-4o", created: "2026-06-08 20:10", turns: 15, tokens: 67800, status: "archived", size: "42.1 KB" },
];

const navItems = [
  { id: "dashboard", label: "仪表盘", icon: BarChart3 },
  { id: "chat", label: "Agent 对话", icon: MessageSquare },
  { id: "orchestrator", label: "编排引擎", icon: Workflow },
  { id: "tools", label: "工具注册表", icon: Wrench },
  { id: "sessions", label: "会话管理", icon: Database },
  { id: "audit", label: "审计日志", icon: Shield },
  { id: "settings", label: "系统配置", icon: Settings },
];

// ---- Helper Components ----

function StatusDot({ status }) {
  const colors = {
    active: COLORS.accentGreen,
    running: COLORS.accentGreen,
    done: COLORS.primary,
    idle: COLORS.accent,
    pending: COLORS.textMuted,
    archived: COLORS.textMuted,
    error: COLORS.accentRed,
    warn: COLORS.accent,
  };
  return (
    <span style={{
      display: "inline-block", width: 8, height: 8, borderRadius: "50%",
      backgroundColor: colors[status] || COLORS.textMuted,
      boxShadow: status === "active" || status === "running" ? `0 0 8px ${colors[status]}60` : "none",
    }} />
  );
}

function Badge({ children, color = COLORS.primary }) {
  return (
    <span style={{
      display: "inline-flex", alignItems: "center", padding: "2px 8px",
      borderRadius: 6, fontSize: 11, fontWeight: 600,
      backgroundColor: color + "20", color: color, border: `1px solid ${color}30`,
    }}>{children}</span>
  );
}

function RiskBadge({ risk }) {
  const map = {
    "Safe": COLORS.accentGreen,
    "Low": COLORS.accentCyan,
    "Medium": COLORS.accent,
    "Safe→Critical": COLORS.accentRed,
  };
  return <Badge color={map[risk] || COLORS.textMuted}>{risk}</Badge>;
}

function Card({ children, style = {} }) {
  return (
    <div style={{
      backgroundColor: COLORS.surface, borderRadius: 12,
      border: `1px solid ${COLORS.border}`, padding: 20, ...style,
    }}>{children}</div>
  );
}

function StatCard({ icon: Icon, label, value, sub, color = COLORS.primary }) {
  return (
    <Card style={{ display: "flex", flexDirection: "column", gap: 8 }}>
      <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
        <div style={{
          width: 36, height: 36, borderRadius: 10,
          backgroundColor: color + "18", display: "flex",
          alignItems: "center", justifyContent: "center",
        }}>
          <Icon size={18} color={color} />
        </div>
        <span style={{ fontSize: 12, color: COLORS.textSecondary }}>{label}</span>
      </div>
      <div style={{ fontSize: 28, fontWeight: 700, color: COLORS.textPrimary }}>{value}</div>
      {sub && <div style={{ fontSize: 11, color: COLORS.textMuted }}>{sub}</div>}
    </Card>
  );
}

function SectionTitle({ icon: Icon, title, subtitle }) {
  return (
    <div style={{ marginBottom: 20 }}>
      <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
        {Icon && <Icon size={20} color={COLORS.primaryLight} />}
        <h2 style={{ fontSize: 20, fontWeight: 700, color: COLORS.textPrimary, margin: 0 }}>{title}</h2>
      </div>
      {subtitle && <p style={{ fontSize: 13, color: COLORS.textMuted, margin: "4px 0 0 30px" }}>{subtitle}</p>}
    </div>
  );
}

// ---- Page: Dashboard ----
function DashboardPage() {
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 24 }}>
      <SectionTitle icon={BarChart3} title="系统概览" subtitle="Orion Agent Framework v0.1.0 — 实时运行状态" />

      <div style={{ display: "grid", gridTemplateColumns: "repeat(4, 1fr)", gap: 16 }}>
        <StatCard icon={MessageSquare} label="对话轮次" value="127" sub="今日 +23 轮" color={COLORS.primary} />
        <StatCard icon={Cpu} label="Token 消耗" value="1.2M" sub="input 680K / output 520K" color={COLORS.accentCyan} />
        <StatCard icon={Wrench} label="工具调用" value="342" sub="成功率 98.2%" color={COLORS.accentGreen} />
        <StatCard icon={Zap} label="缓存命中率" value="78%" sub="L1: 68% / L2: 22%" color={COLORS.accent} />
      </div>

      <div style={{ display: "grid", gridTemplateColumns: "2fr 1fr", gap: 16 }}>
        <Card>
          <div style={{ fontSize: 14, fontWeight: 600, color: COLORS.textPrimary, marginBottom: 16 }}>Token 使用趋势</div>
          <ResponsiveContainer width="100%" height={200}>
            <AreaChart data={tokenUsageData}>
              <defs>
                <linearGradient id="gInput" x1="0" y1="0" x2="0" y2="1">
                  <stop offset="0%" stopColor={COLORS.primary} stopOpacity={0.3} />
                  <stop offset="100%" stopColor={COLORS.primary} stopOpacity={0} />
                </linearGradient>
                <linearGradient id="gOutput" x1="0" y1="0" x2="0" y2="1">
                  <stop offset="0%" stopColor={COLORS.accentCyan} stopOpacity={0.3} />
                  <stop offset="100%" stopColor={COLORS.accentCyan} stopOpacity={0} />
                </linearGradient>
              </defs>
              <XAxis dataKey="t" tick={{ fontSize: 11, fill: COLORS.textMuted }} axisLine={false} tickLine={false} />
              <YAxis tick={{ fontSize: 11, fill: COLORS.textMuted }} axisLine={false} tickLine={false} />
              <Tooltip
                contentStyle={{ backgroundColor: COLORS.surface, border: `1px solid ${COLORS.border}`, borderRadius: 8, fontSize: 12 }}
                labelStyle={{ color: COLORS.textSecondary }}
              />
              <Area type="monotone" dataKey="input" stroke={COLORS.primary} fill="url(#gInput)" strokeWidth={2} name="Input" />
              <Area type="monotone" dataKey="output" stroke={COLORS.accentCyan} fill="url(#gOutput)" strokeWidth={2} name="Output" />
            </AreaChart>
          </ResponsiveContainer>
        </Card>

        <Card>
          <div style={{ fontSize: 14, fontWeight: 600, color: COLORS.textPrimary, marginBottom: 16 }}>工具调用分布</div>
          <ResponsiveContainer width="100%" height={200}>
            <PieChart>
              <Pie data={toolDistData} cx="50%" cy="50%" innerRadius={50} outerRadius={80} dataKey="value" stroke="none">
                {toolDistData.map((entry, i) => (
                  <Cell key={i} fill={entry.color} />
                ))}
              </Pie>
              <Tooltip
                contentStyle={{ backgroundColor: COLORS.surface, border: `1px solid ${COLORS.border}`, borderRadius: 8, fontSize: 12 }}
              />
            </PieChart>
          </ResponsiveContainer>
          <div style={{ display: "flex", flexWrap: "wrap", gap: 8, justifyContent: "center" }}>
            {toolDistData.slice(0, 5).map((t) => (
              <span key={t.name} style={{ fontSize: 11, color: COLORS.textSecondary, display: "flex", alignItems: "center", gap: 4 }}>
                <span style={{ width: 8, height: 8, borderRadius: 2, backgroundColor: t.color }} />
                {t.name}
              </span>
            ))}
          </div>
        </Card>
      </div>

      <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 16 }}>
        <Card>
          <div style={{ fontSize: 14, fontWeight: 600, color: COLORS.textPrimary, marginBottom: 16 }}>架构层级</div>
          {[
            { name: "Gateway 元系统层", desc: "命令路由 · 日志 · 事件总线 · 审计", status: "active", modules: 4 },
            { name: "UI 层", desc: "REPL · TUI · Web API", status: "active", modules: 3 },
            { name: "编排层 Orchestrator", desc: "Coordinator · Worker · Plan (DAG)", status: "active", modules: 3 },
            { name: "Agent 运行时", desc: "Runtime · Lanes · Protocol", status: "active", modules: 3 },
            { name: "核心层 Core", desc: "Provider · Loop · Context · Guardrail · Cache", status: "active", modules: 5 },
            { name: "工具层 Tools", desc: "17 个内置工具 + MCP 扩展", status: "active", modules: 17 },
            { name: "基础设施", desc: "Session · Index · Plugin · Model", status: "active", modules: 4 },
          ].map((layer, i) => (
            <div key={i} style={{
              display: "flex", alignItems: "center", gap: 12, padding: "8px 12px",
              borderRadius: 8, marginBottom: 6,
              backgroundColor: i % 2 === 0 ? COLORS.surfaceHover : "transparent",
            }}>
              <StatusDot status={layer.status} />
              <div style={{ flex: 1 }}>
                <div style={{ fontSize: 13, fontWeight: 600, color: COLORS.textPrimary }}>{layer.name}</div>
                <div style={{ fontSize: 11, color: COLORS.textMuted }}>{layer.desc}</div>
              </div>
              <Badge color={COLORS.primaryLight}>{layer.modules} 模块</Badge>
            </div>
          ))}
        </Card>

        <Card>
          <div style={{ fontSize: 14, fontWeight: 600, color: COLORS.textPrimary, marginBottom: 16 }}>实时活动流</div>
          <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
            {auditEvents.slice(0, 8).map((evt) => (
              <div key={evt.id} style={{
                display: "flex", gap: 10, padding: "6px 0",
                borderBottom: `1px solid ${COLORS.border}`,
              }}>
                <span style={{ fontSize: 11, color: COLORS.textMuted, fontFamily: "monospace", whiteSpace: "nowrap" }}>{evt.time}</span>
                <Badge color={
                  evt.level === "error" ? COLORS.accentRed :
                  evt.level === "warn" ? COLORS.accent : COLORS.primaryLight
                }>{evt.type}</Badge>
                <span style={{ fontSize: 12, color: COLORS.textSecondary, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{evt.detail}</span>
              </div>
            ))}
          </div>
        </Card>
      </div>
    </div>
  );
}

// ---- Page: Chat ----
function ChatPage() {
  const [inputVal, setInputVal] = useState("");
  const [streaming, setStreaming] = useState(false);
  const [streamText, setStreamText] = useState("");
  const chatRef = useRef(null);
  const fullText = "正在通过 `Provider.stream()` 流式调用 DeepSeek...\n\n已完成代码安全审查，发现 2 个潜在风险点：\n1. `bash` 工具执行命令时缺少完整的路径白名单校验\n2. `write` 工具对工作区外的路径检查存在 TOCTOU 竞态条件\n\n建议：增加 `ExecPolicy` 白名单覆盖，并在 `workspace.rs` 中引入文件锁。";

  useEffect(() => {
    if (chatRef.current) {
      chatRef.current.scrollTop = chatRef.current.scrollHeight;
    }
  }, [streamText]);

  const handleSend = () => {
    if (!inputVal.trim()) return;
    setStreaming(true);
    setStreamText("");
    let i = 0;
    const interval = setInterval(() => {
      if (i < fullText.length) {
        setStreamText(fullText.slice(0, i + 1));
        i++;
      } else {
        clearInterval(interval);
        setStreaming(false);
      }
    }, 20);
    setInputVal("");
  };

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "calc(100vh - 80px)" }}>
      <SectionTitle icon={MessageSquare} title="Agent 对话" subtitle="基于 run_simple_loop() 的流式对话 — 支持工具调用、上下文压缩、缓存命中" />

      <div ref={chatRef} style={{
        flex: 1, overflowY: "auto", display: "flex", flexDirection: "column", gap: 16,
        padding: "0 4px 16px 4px",
      }}>
        {chatMessages.map((msg, i) => (
          <div key={i}>
            <div style={{
              display: "flex", gap: 12,
              flexDirection: msg.role === "user" ? "row-reverse" : "row",
            }}>
              <div style={{
                width: 32, height: 32, borderRadius: 8, flexShrink: 0,
                backgroundColor: msg.role === "user" ? COLORS.accent + "30" : COLORS.primary + "30",
                display: "flex", alignItems: "center", justifyContent: "center",
              }}>
                {msg.role === "user" ? <Users size={16} color={COLORS.accent} /> : <Bot size={16} color={COLORS.primaryLight} />}
              </div>
              <div style={{
                maxWidth: "75%", padding: "12px 16px", borderRadius: 12,
                backgroundColor: msg.role === "user" ? COLORS.primary + "15" : COLORS.surfaceHover,
                border: `1px solid ${msg.role === "user" ? COLORS.primary + "30" : COLORS.border}`,
              }}>
                {msg.thinking && (
                  <div style={{
                    marginBottom: 10, padding: "8px 12px", borderRadius: 8,
                    backgroundColor: COLORS.accentPurple + "12",
                    border: `1px solid ${COLORS.accentPurple}25`,
                    fontSize: 12, color: COLORS.accentPurple, fontStyle: "italic",
                  }}>
                    <div style={{ display: "flex", alignItems: "center", gap: 6, marginBottom: 4 }}>
                      <Sparkles size={12} /> <span style={{ fontWeight: 600 }}>Thinking</span>
                    </div>
                    {msg.thinking}
                  </div>
                )}
                <div style={{ fontSize: 13, color: COLORS.textPrimary, lineHeight: 1.6, whiteSpace: "pre-wrap" }}>{msg.content}</div>
                {msg.tools && (
                  <div style={{ marginTop: 12, display: "flex", flexDirection: "column", gap: 6 }}>
                    {msg.tools.map((tool, j) => (
                      <div key={j} style={{
                        display: "flex", alignItems: "center", gap: 8, padding: "6px 10px",
                        borderRadius: 8, backgroundColor: COLORS.bg,
                        border: `1px solid ${COLORS.border}`,
                        fontSize: 12,
                      }}>
                        <Wrench size={12} color={COLORS.accentCyan} />
                        <span style={{ fontWeight: 600, color: COLORS.accentCyan }}>{tool.name}</span>
                        <span style={{ color: COLORS.textMuted, fontFamily: "monospace", fontSize: 11 }}>{tool.input}</span>
                        <span style={{ marginLeft: "auto" }}>
                          {tool.status === "done" ? (
                            <Badge color={COLORS.accentGreen}><Check size={10} /> {tool.result}</Badge>
                          ) : (
                            <Badge color={COLORS.accent}>
                              <RefreshCw size={10} style={{ animation: "spin 1s linear infinite" }} /> 执行中...
                            </Badge>
                          )}
                        </span>
                      </div>
                    ))}
                  </div>
                )}
              </div>
            </div>
          </div>
        ))}

        {streaming && (
          <div style={{ display: "flex", gap: 12 }}>
            <div style={{
              width: 32, height: 32, borderRadius: 8, flexShrink: 0,
              backgroundColor: COLORS.primary + "30",
              display: "flex", alignItems: "center", justifyContent: "center",
            }}>
              <Bot size={16} color={COLORS.primaryLight} />
            </div>
            <div style={{
              maxWidth: "75%", padding: "12px 16px", borderRadius: 12,
              backgroundColor: COLORS.surfaceHover,
              border: `1px solid ${COLORS.border}`,
              fontSize: 13, color: COLORS.textPrimary, lineHeight: 1.6, whiteSpace: "pre-wrap",
            }}>
              {streamText}
              <span style={{
                display: "inline-block", width: 2, height: 14, backgroundColor: COLORS.primaryLight,
                marginLeft: 2, animation: "blink 0.8s infinite",
                verticalAlign: "middle",
              }} />
            </div>
          </div>
        )}
      </div>

      <div style={{
        display: "flex", gap: 10, padding: "12px 0 0 0",
        borderTop: `1px solid ${COLORS.border}`,
      }}>
        <div style={{
          display: "flex", gap: 6, alignItems: "center", padding: "0 12px",
          borderRadius: 8, backgroundColor: COLORS.surfaceHover,
          border: `1px solid ${COLORS.border}`,
        }}>
          <Bot size={14} color={COLORS.primaryLight} />
          <span style={{ fontSize: 12, color: COLORS.textSecondary }}>deepseek-chat</span>
        </div>
        <input
          value={inputVal}
          onChange={(e) => setInputVal(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && handleSend()}
          placeholder="输入消息... (支持 /model、/think、/compact 等斜杠命令)"
          style={{
            flex: 1, padding: "10px 16px", borderRadius: 10,
            backgroundColor: COLORS.surface, border: `1px solid ${COLORS.border}`,
            color: COLORS.textPrimary, fontSize: 13, outline: "none",
          }}
        />
        <button
          onClick={handleSend}
          style={{
            padding: "10px 20px", borderRadius: 10, border: "none",
            backgroundColor: COLORS.primary, color: "#fff", fontSize: 13,
            fontWeight: 600, cursor: "pointer", display: "flex", alignItems: "center", gap: 6,
          }}
        >
          <Send size={14} /> 发送
        </button>
      </div>

      <style>{`
        @keyframes spin { from { transform: rotate(0deg); } to { transform: rotate(360deg); } }
        @keyframes blink { 0%, 100% { opacity: 1; } 50% { opacity: 0; } }
      `}</style>
    </div>
  );
}

// ---- Page: Orchestrator ----
function OrchestratorPage() {
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 24 }}>
      <SectionTitle icon={Workflow} title="多 Agent 编排引擎" subtitle="Coordinator 通过 LLM 自动拆解任务为 DAG，Worker 独立执行子任务" />

      <div style={{ display: "grid", gridTemplateColumns: "1fr 2fr", gap: 16 }}>
        <Card>
          <div style={{ fontSize: 14, fontWeight: 600, color: COLORS.textPrimary, marginBottom: 16 }}>编排配置</div>
          {[
            { label: "编排模式", value: "Sequential → Parallel", icon: Layers },
            { label: "协调模型", value: "deepseek-chat", icon: Cpu },
            { label: "最大 Worker", value: "3", icon: Users },
            { label: "最大轮次", value: "10", icon: Target },
            { label: "Token 预算", value: "128,000", icon: Gauge },
          ].map((item, i) => (
            <div key={i} style={{
              display: "flex", alignItems: "center", gap: 10, padding: "10px 12px",
              borderBottom: i < 4 ? `1px solid ${COLORS.border}` : "none",
            }}>
              <item.icon size={14} color={COLORS.textMuted} />
              <span style={{ fontSize: 12, color: COLORS.textSecondary, flex: 1 }}>{item.label}</span>
              <span style={{ fontSize: 12, fontWeight: 600, color: COLORS.textPrimary }}>{item.value}</span>
            </div>
          ))}

          <div style={{ marginTop: 16, padding: "12px", borderRadius: 8, backgroundColor: COLORS.bg, border: `1px solid ${COLORS.border}` }}>
            <div style={{ fontSize: 11, fontWeight: 600, color: COLORS.textMuted, marginBottom: 8 }}>Worker 类型</div>
            {[
              { type: "Search", desc: "只读探索", color: COLORS.accentCyan, tools: "read, glob, grep, symbol" },
              { type: "Code", desc: "读写+执行", color: COLORS.accentGreen, tools: "read, write, bash, edit" },
              { type: "Review", desc: "只读审查", color: COLORS.accentPurple, tools: "read, grep, symbol, bash(ro)" },
            ].map((w, i) => (
              <div key={i} style={{ display: "flex", alignItems: "center", gap: 8, padding: "6px 0" }}>
                <span style={{ width: 8, height: 8, borderRadius: 2, backgroundColor: w.color }} />
                <span style={{ fontSize: 12, fontWeight: 600, color: w.color }}>{w.type}</span>
                <span style={{ fontSize: 11, color: COLORS.textMuted }}>{w.desc}</span>
              </div>
            ))}
          </div>
        </Card>

        <Card>
          <div style={{ fontSize: 14, fontWeight: 600, color: COLORS.textPrimary, marginBottom: 16 }}>任务 DAG — 实时执行状态</div>
          <div style={{ display: "flex", alignItems: "center", gap: 8, marginBottom: 20 }}>
            <div style={{
              padding: "8px 16px", borderRadius: 10,
              backgroundColor: COLORS.primary + "20",
              border: `1px solid ${COLORS.primary}40`,
              display: "flex", alignItems: "center", gap: 8,
            }}>
              <Sparkles size={14} color={COLORS.primaryLight} />
              <span style={{ fontSize: 13, fontWeight: 600, color: COLORS.primaryLight }}>
                {orchestrationDAG.coordinator.name}
              </span>
              <Badge color={COLORS.accentGreen}>active</Badge>
            </div>
            <span style={{ fontSize: 12, color: COLORS.textMuted }}>model: {orchestrationDAG.coordinator.model}</span>
          </div>

          <div style={{ position: "relative" }}>
            {orchestrationDAG.tasks.map((task, i) => (
              <div key={task.id} style={{ display: "flex", alignItems: "flex-start", gap: 16, marginBottom: 4 }}>
                <div style={{ display: "flex", flexDirection: "column", alignItems: "center", width: 40 }}>
                  <div style={{
                    width: 36, height: 36, borderRadius: "50%",
                    backgroundColor: task.status === "done" ? COLORS.accentGreen + "20" :
                      task.status === "running" ? COLORS.accent + "20" : COLORS.surfaceHover,
                    border: `2px solid ${task.status === "done" ? COLORS.accentGreen :
                      task.status === "running" ? COLORS.accent : COLORS.border}`,
                    display: "flex", alignItems: "center", justifyContent: "center",
                    fontSize: 12, fontWeight: 700,
                    color: task.status === "done" ? COLORS.accentGreen :
                      task.status === "running" ? COLORS.accent : COLORS.textMuted,
                  }}>
                    {task.status === "done" ? <Check size={16} /> : task.id}
                  </div>
                  {i < orchestrationDAG.tasks.length - 1 && (
                    <div style={{ width: 2, height: 28, backgroundColor: COLORS.border, marginTop: 4 }} />
                  )}
                </div>
                <div style={{
                  flex: 1, padding: "10px 14px", borderRadius: 10,
                  backgroundColor: COLORS.surfaceHover,
                  border: `1px solid ${task.status === "running" ? COLORS.accent + "40" : COLORS.border}`,
                  marginBottom: 8,
                }}>
                  <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
                    <span style={{ fontSize: 14, fontWeight: 600, color: COLORS.textPrimary }}>{task.name}</span>
                    <StatusDot status={task.status} />
                    <Badge color={
                      task.status === "done" ? COLORS.accentGreen :
                      task.status === "running" ? COLORS.accent : COLORS.textMuted
                    }>{task.status}</Badge>
                    <span style={{ marginLeft: "auto", fontSize: 11, color: COLORS.textMuted }}>{task.duration}</span>
                  </div>
                  <div style={{ display: "flex", gap: 12, marginTop: 6 }}>
                    <span style={{ fontSize: 11, color: COLORS.textMuted }}>Worker: {task.worker}</span>
                    {task.deps.length > 0 && (
                      <span style={{ fontSize: 11, color: COLORS.textMuted }}>
                        依赖: {task.deps.join(", ")}
                      </span>
                    )}
                  </div>
                </div>
              </div>
            ))}
          </div>
        </Card>
      </div>

      <Card>
        <div style={{ fontSize: 14, fontWeight: 600, color: COLORS.textPrimary, marginBottom: 16 }}>编排执行流程</div>
        <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between", gap: 8, flexWrap: "wrap" }}>
          {[
            { label: "用户输入", sub: "自然语言任务", color: COLORS.accent },
            { label: "Coordinator", sub: "LLM 任务拆解", color: COLORS.primary },
            { label: "TaskPlan", sub: "DAG 依赖解析", color: COLORS.accentCyan },
            { label: "Worker 分派", sub: "独立上下文执行", color: COLORS.accentGreen },
            { label: "StepObserver", sub: "结果观察/重试", color: COLORS.accentPurple },
            { label: "结果汇总", sub: "合并子任务输出", color: COLORS.accent },
          ].map((step, i, arr) => (
            <div key={i} style={{ display: "flex", alignItems: "center", gap: 8 }}>
              <div style={{
                padding: "10px 16px", borderRadius: 10,
                backgroundColor: step.color + "15",
                border: `1px solid ${step.color}30`,
                textAlign: "center", minWidth: 100,
              }}>
                <div style={{ fontSize: 13, fontWeight: 600, color: step.color }}>{step.label}</div>
                <div style={{ fontSize: 10, color: COLORS.textMuted, marginTop: 2 }}>{step.sub}</div>
              </div>
              {i < arr.length - 1 && <ArrowRight size={16} color={COLORS.textMuted} />}
            </div>
          ))}
        </div>
      </Card>
    </div>
  );
}

// ---- Page: Tools ----
function ToolsPage() {
  const [selectedCategory, setSelectedCategory] = useState("全部");
  const categories = ["全部", "核心", "搜索", "代码智能", "编排", "外部", "交互"];
  const filtered = selectedCategory === "全部" ? tools : tools.filter((t) => t.category === selectedCategory);

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 24 }}>
      <SectionTitle icon={Wrench} title="工具注册表" subtitle="17 个内置工具 — 按需组合，支持 MCP 协议扩展" />

      <div style={{ display: "flex", gap: 8 }}>
        {categories.map((cat) => (
          <button
            key={cat}
            onClick={() => setSelectedCategory(cat)}
            style={{
              padding: "6px 14px", borderRadius: 8, border: "none", cursor: "pointer",
              fontSize: 12, fontWeight: 600,
              backgroundColor: selectedCategory === cat ? COLORS.primary : COLORS.surfaceHover,
              color: selectedCategory === cat ? "#fff" : COLORS.textSecondary,
            }}
          >{cat}</button>
        ))}
      </div>

      <div style={{ display: "grid", gridTemplateColumns: "repeat(3, 1fr)", gap: 12 }}>
        {filtered.map((tool) => (
          <Card key={tool.name} style={{ padding: 16, cursor: "pointer", transition: "border-color 0.2s" }}>
            <div style={{ display: "flex", alignItems: "center", gap: 8, marginBottom: 8 }}>
              <div style={{
                width: 28, height: 28, borderRadius: 8,
                backgroundColor: COLORS.primary + "18",
                display: "flex", alignItems: "center", justifyContent: "center",
              }}>
                <Wrench size={14} color={COLORS.primaryLight} />
              </div>
              <span style={{ fontSize: 14, fontWeight: 700, color: COLORS.textPrimary, fontFamily: "monospace" }}>{tool.name}</span>
              <div style={{ marginLeft: "auto" }}><RiskBadge risk={tool.risk} /></div>
            </div>
            <div style={{ fontSize: 12, color: COLORS.textSecondary, marginBottom: 8 }}>{tool.desc}</div>
            <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
              <Badge color={COLORS.textMuted}>{tool.category}</Badge>
              <span style={{ fontSize: 11, color: COLORS.textMuted }}>调用 {tool.count} 次</span>
            </div>
          </Card>
        ))}
      </div>

      <Card>
        <div style={{ fontSize: 14, fontWeight: 600, color: COLORS.textPrimary, marginBottom: 16 }}>工具执行流水线 (AOP)</div>
        <div style={{ display: "flex", alignItems: "center", justifyContent: "center", gap: 6, flexWrap: "wrap" }}>
          {[
            { label: "ToolCall 请求", color: COLORS.accent },
            { label: "ExecPolicy 白名单", color: COLORS.accentRed },
            { label: "GuardrailChain 检查", color: COLORS.accentRed },
            { label: "HookEngine before", color: COLORS.accentPurple },
            { label: "AOP 路径规范化", color: COLORS.accentCyan },
            { label: "执行工具", color: COLORS.accentGreen },
            { label: "HookEngine after", color: COLORS.accentPurple },
            { label: "StepObserver", color: COLORS.primary },
          ].map((step, i, arr) => (
            <div key={i} style={{ display: "flex", alignItems: "center", gap: 6 }}>
              <div style={{
                padding: "6px 12px", borderRadius: 8,
                backgroundColor: step.color + "15",
                border: `1px solid ${step.color}30`,
                fontSize: 11, fontWeight: 600, color: step.color,
              }}>{step.label}</div>
              {i < arr.length - 1 && <ChevronRight size={14} color={COLORS.textMuted} />}
            </div>
          ))}
        </div>
      </Card>
    </div>
  );
}

// ---- Page: Sessions ----
function SessionsPage() {
  const [selectedSession, setSelectedSession] = useState(sessions[0]);
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 24 }}>
      <SectionTitle icon={Database} title="会话管理" subtitle="SQLite 持久化 + JSONL 转录 + 文件回滚快照" />

      <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 16 }}>
        <Card style={{ padding: 0, overflow: "hidden" }}>
          <div style={{ padding: "14px 16px", borderBottom: `1px solid ${COLORS.border}`, display: "flex", alignItems: "center", gap: 8 }}>
            <Database size={14} color={COLORS.primaryLight} />
            <span style={{ fontSize: 14, fontWeight: 600, color: COLORS.textPrimary }}>会话列表</span>
            <Badge color={COLORS.primaryLight}>{sessions.length} 个会话</Badge>
          </div>
          {sessions.map((sess) => (
            <div
              key={sess.id}
              onClick={() => setSelectedSession(sess)}
              style={{
                padding: "12px 16px", cursor: "pointer",
                borderBottom: `1px solid ${COLORS.border}`,
                backgroundColor: selectedSession.id === sess.id ? COLORS.surfaceHover : "transparent",
              }}
            >
              <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
                <StatusDot status={sess.status} />
                <span style={{ fontSize: 13, fontWeight: 600, color: COLORS.textPrimary, fontFamily: "monospace" }}>{sess.id}</span>
                <Badge color={
                  sess.status === "active" ? COLORS.accentGreen :
                  sess.status === "idle" ? COLORS.accent : COLORS.textMuted
                }>{sess.status}</Badge>
              </div>
              <div style={{ display: "flex", gap: 16, marginTop: 6, fontSize: 11, color: COLORS.textMuted }}>
                <span>model: {sess.model}</span>
                <span>{sess.turns} 轮</span>
                <span>{(sess.tokens / 1000).toFixed(1)}K tokens</span>
                <span>{sess.size}</span>
              </div>
            </div>
          ))}
        </Card>

        <Card>
          <div style={{ fontSize: 14, fontWeight: 600, color: COLORS.textPrimary, marginBottom: 16 }}>
            会话详情 — <span style={{ fontFamily: "monospace", color: COLORS.primaryLight }}>{selectedSession.id}</span>
          </div>
          {[
            { label: "会话 ID", value: selectedSession.id },
            { label: "模型", value: selectedSession.model },
            { label: "创建时间", value: selectedSession.created },
            { label: "对话轮次", value: selectedSession.turns + " 轮" },
            { label: "Token 消耗", value: selectedSession.tokens.toLocaleString() },
            { label: "存储大小", value: selectedSession.size },
            { label: "状态", value: selectedSession.status },
          ].map((item, i) => (
            <div key={i} style={{
              display: "flex", alignItems: "center", padding: "10px 0",
              borderBottom: `1px solid ${COLORS.border}`,
            }}>
              <span style={{ fontSize: 12, color: COLORS.textMuted, width: 100 }}>{item.label}</span>
              <span style={{ fontSize: 13, fontWeight: 600, color: COLORS.textPrimary, fontFamily: "monospace" }}>{item.value}</span>
            </div>
          ))}

          <div style={{ marginTop: 16 }}>
            <div style={{ fontSize: 12, fontWeight: 600, color: COLORS.textMuted, marginBottom: 8 }}>存储结构</div>
            <div style={{
              padding: 12, borderRadius: 8, backgroundColor: COLORS.bg,
              border: `1px solid ${COLORS.border}`, fontFamily: "monospace", fontSize: 11,
              color: COLORS.textSecondary, lineHeight: 1.8,
            }}>
              <div>.orion/sessions/</div>
              <div style={{ paddingLeft: 16 }}>├── {selectedSession.id}/</div>
              <div style={{ paddingLeft: 32 }}>├── transcript.jsonl <span style={{ color: COLORS.textMuted }}># 对话转录</span></div>
              <div style={{ paddingLeft: 32 }}>├── index.json <span style={{ color: COLORS.textMuted }}># 快速索引</span></div>
              <div style={{ paddingLeft: 32 }}>├── snapshots/ <span style={{ color: COLORS.textMuted }}># 文件快照</span></div>
              <div style={{ paddingLeft: 48 }}>└── loop.rs.snap.001</div>
              <div style={{ paddingLeft: 32 }}>└── audit.jsonl <span style={{ color: COLORS.textMuted }}># 审计日志</span></div>
            </div>
          </div>

          <div style={{ display: "flex", gap: 8, marginTop: 16 }}>
            <button style={{
              flex: 1, padding: "8px 12px", borderRadius: 8, border: "none",
              backgroundColor: COLORS.primary, color: "#fff", fontSize: 12,
              fontWeight: 600, cursor: "pointer",
            }}>恢复会话 (--resume)</button>
            <button style={{
              flex: 1, padding: "8px 12px", borderRadius: 8,
              border: `1px solid ${COLORS.border}`, backgroundColor: "transparent",
              color: COLORS.textSecondary, fontSize: 12, fontWeight: 600, cursor: "pointer",
            }}>导出 JSONL</button>
          </div>
        </Card>
      </div>

      <Card>
        <div style={{ fontSize: 14, fontWeight: 600, color: COLORS.textPrimary, marginBottom: 16 }}>跨 Session 记忆系统</div>
        <div style={{ display: "grid", gridTemplateColumns: "repeat(5, 1fr)", gap: 10 }}>
          {[
            { cat: "UserPreference", count: 8, color: COLORS.accent, desc: "用户偏好" },
            { cat: "ProjectFact", count: 15, color: COLORS.accentCyan, desc: "项目事实" },
            { cat: "CodePattern", count: 12, color: COLORS.accentGreen, desc: "代码模式" },
            { cat: "Decision", count: 6, color: COLORS.accentPurple, desc: "关键决策" },
            { cat: "Constraint", count: 4, color: COLORS.accentRed, desc: "约束条件" },
          ].map((mem) => (
            <div key={mem.cat} style={{
              padding: 12, borderRadius: 10,
              backgroundColor: mem.color + "10",
              border: `1px solid ${mem.color}25`,
              textAlign: "center",
            }}>
              <div style={{ fontSize: 22, fontWeight: 700, color: mem.color }}>{mem.count}</div>
              <div style={{ fontSize: 12, fontWeight: 600, color: COLORS.textPrimary, marginTop: 4 }}>{mem.desc}</div>
              <div style={{ fontSize: 10, color: COLORS.textMuted, fontFamily: "monospace" }}>{mem.cat}</div>
            </div>
          ))}
        </div>
      </Card>
    </div>
  );
}

// ---- Page: Audit ----
function AuditPage() {
  const [filterType, setFilterType] = useState("全部");
  const types = ["全部", "SessionStart", "LlmRequest", "ToolCall", "FileOperation", "SecurityEvent", "Error", "ConfigChange"];
  const filtered = filterType === "全部" ? auditEvents : auditEvents.filter((e) => e.type === filterType);

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 24 }}>
      <SectionTitle icon={Shield} title="审计日志" subtitle="9 种事件类型 · JSONL 格式 · 敏感信息自动脱敏" />

      <div style={{ display: "grid", gridTemplateColumns: "repeat(4, 1fr)", gap: 12 }}>
        <StatCard icon={Activity} label="今日事件" value="1,247" sub="9 种类型" color={COLORS.primary} />
        <StatCard icon={Shield} label="安全事件" value="3" sub="全部已拦截" color={COLORS.accentRed} />
        <StatCard icon={Wrench} label="工具调用" value="342" sub="成功率 98.2%" color={COLORS.accentGreen} />
        <StatCard icon={Cpu} label="LLM 请求" value="89" sub="平均 2.1s/req" color={COLORS.accentCyan} />
      </div>

      <div style={{ display: "flex", gap: 8, flexWrap: "wrap" }}>
        {types.map((type) => (
          <button
            key={type}
            onClick={() => setFilterType(type)}
            style={{
              padding: "5px 12px", borderRadius: 6, border: "none", cursor: "pointer",
              fontSize: 11, fontWeight: 600,
              backgroundColor: filterType === type ? COLORS.primary : COLORS.surfaceHover,
              color: filterType === type ? "#fff" : COLORS.textSecondary,
            }}
          >{type}</button>
        ))}
      </div>

      <Card style={{ padding: 0, overflow: "hidden" }}>
        <div style={{
          display: "grid", gridTemplateColumns: "60px 140px 1fr 100px",
          padding: "10px 16px", borderBottom: `1px solid ${COLORS.border}`,
          fontSize: 11, fontWeight: 600, color: COLORS.textMuted,
        }}>
          <span>时间</span>
          <span>事件类型</span>
          <span>详情</span>
          <span>级别</span>
        </div>
        {filtered.map((evt) => (
          <div key={evt.id} style={{
            display: "grid", gridTemplateColumns: "60px 140px 1fr 100px",
            padding: "10px 16px", borderBottom: `1px solid ${COLORS.border}`,
            fontSize: 12, alignItems: "center",
            backgroundColor: evt.level === "error" ? COLORS.accentRed + "08" :
              evt.level === "warn" ? COLORS.accent + "08" : "transparent",
          }}>
            <span style={{ fontFamily: "monospace", color: COLORS.textMuted, fontSize: 11 }}>{evt.time}</span>
            <Badge color={
              evt.type === "SecurityEvent" ? COLORS.accentRed :
              evt.type === "Error" ? COLORS.accentRed :
              evt.type === "ToolCall" ? COLORS.accentCyan :
              evt.type === "LlmRequest" ? COLORS.accentGreen :
              evt.type === "ConfigChange" ? COLORS.accentPurple :
              COLORS.primaryLight
            }>{evt.type}</Badge>
            <span style={{ color: COLORS.textSecondary, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{evt.detail}</span>
            <span>
              <Badge color={
                evt.level === "error" ? COLORS.accentRed :
                evt.level === "warn" ? COLORS.accent : COLORS.accentGreen
              }>{evt.level}</Badge>
            </span>
          </div>
        ))}
      </Card>

      <Card>
        <div style={{ fontSize: 14, fontWeight: 600, color: COLORS.textPrimary, marginBottom: 12 }}>审计数据格式 (JSONL)</div>
        <div style={{
          padding: 16, borderRadius: 8, backgroundColor: COLORS.bg,
          border: `1px solid ${COLORS.border}`, fontFamily: "monospace",
          fontSize: 11, color: COLORS.textSecondary, lineHeight: 1.8,
          overflowX: "auto",
        }}>
          <div style={{ color: COLORS.textMuted }}>{"// 每行一条 JSON 记录，便于流式写入和日志分析"}</div>
          <div>{'{'}</div>
          <div style={{ paddingLeft: 16 }}><span style={{ color: COLORS.accentCyan }}>"timestamp"</span>: <span style={{ color: COLORS.accentGreen }}>"2026-06-10T14:32:05.123Z"</span>,</div>
          <div style={{ paddingLeft: 16 }}><span style={{ color: COLORS.accentCyan }}>"session_id"</span>: <span style={{ color: COLORS.accentGreen }}>"sess_0x7f3a"</span>,</div>
          <div style={{ paddingLeft: 16 }}><span style={{ color: COLORS.accentCyan }}>"event_type"</span>: <span style={{ color: COLORS.accentGreen }}>"ToolCall"</span>,</div>
          <div style={{ paddingLeft: 16 }}><span style={{ color: COLORS.accentCyan }}>"tool_name"</span>: <span style={{ color: COLORS.accentGreen }}>"bash"</span>,</div>
          <div style={{ paddingLeft: 16 }}><span style={{ color: COLORS.accentCyan }}>"input_summary"</span>: <span style={{ color: COLORS.accentGreen }}>"cargo build --release"</span>,</div>
          <div style={{ paddingLeft: 16 }}><span style={{ color: COLORS.accentCyan }}>"risk_level"</span>: <span style={{ color: COLORS.accentGreen }}>"Low"</span>,</div>
          <div style={{ paddingLeft: 16 }}><span style={{ color: COLORS.accentCyan }}>"success"</span>: <span style={{ color: COLORS.primary }}>true</span>,</div>
          <div style={{ paddingLeft: 16 }}><span style={{ color: COLORS.accentCyan }}>"duration_ms"</span>: <span style={{ color: COLORS.primary }}>3200</span>,</div>
          <div style={{ paddingLeft: 16 }}><span style={{ color: COLORS.accentCyan }}>"api_key"</span>: <span style={{ color: COLORS.accentRed }}>"sk-***"</span> <span style={{ color: COLORS.textMuted }}>{"// 自动脱敏"}</span></div>
          <div>{"}"}</div>
        </div>
      </Card>
    </div>
  );
}

// ---- Page: Settings ----
function SettingsPage() {
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 24 }}>
      <SectionTitle icon={Settings} title="系统配置" subtitle="YAML 配置 + 环境变量替换 + 模型注册表" />

      <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 16 }}>
        <Card>
          <div style={{ fontSize: 14, fontWeight: 600, color: COLORS.textPrimary, marginBottom: 16 }}>Provider 配置</div>
          {[
            { label: "类型", value: "OpenAI Compatible", status: "active" },
            { label: "API Base", value: "https://api.deepseek.com/v1", status: "active" },
            { label: "模型", value: "deepseek-chat", status: "active" },
            { label: "Thinking Mode", value: "disabled", status: "idle" },
            { label: "API Key", value: "sk-*** (已脱敏)", status: "active" },
          ].map((item, i) => (
            <div key={i} style={{
              display: "flex", alignItems: "center", padding: "10px 0",
              borderBottom: `1px solid ${COLORS.border}`,
            }}>
              <StatusDot status={item.status} />
              <span style={{ fontSize: 12, color: COLORS.textMuted, width: 100, marginLeft: 8 }}>{item.label}</span>
              <span style={{ fontSize: 13, fontWeight: 500, color: COLORS.textPrimary, fontFamily: "monospace" }}>{item.value}</span>
            </div>
          ))}

          <div style={{ marginTop: 16, fontSize: 12, fontWeight: 600, color: COLORS.textMuted, marginBottom: 8 }}>支持的 Provider</div>
          <div style={{ display: "flex", flexWrap: "wrap", gap: 6 }}>
            {["OpenAI", "Anthropic", "DeepSeek", "Qwen", "Ollama", "Custom"].map((p) => (
              <Badge key={p} color={COLORS.accentCyan}>{p}</Badge>
            ))}
          </div>
        </Card>

        <Card>
          <div style={{ fontSize: 14, fontWeight: 600, color: COLORS.textPrimary, marginBottom: 16 }}>缓存系统</div>
          <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr 1fr", gap: 10, marginBottom: 16 }}>
            {[
              { label: "L1 工具缓存", max: "2,048", ttl: "300s", color: COLORS.primary },
              { label: "L2 上下文缓存", max: "128", ttl: "600s", color: COLORS.accentCyan },
              { label: "文件缓存", max: "∞", ttl: "mtime", color: COLORS.accentGreen },
            ].map((cache) => (
              <div key={cache.label} style={{
                padding: 12, borderRadius: 10,
                backgroundColor: cache.color + "10",
                border: `1px solid ${cache.color}25`,
                textAlign: "center",
              }}>
                <div style={{ fontSize: 11, fontWeight: 600, color: cache.color }}>{cache.label}</div>
                <div style={{ fontSize: 20, fontWeight: 700, color: COLORS.textPrimary, marginTop: 4 }}>{cache.max}</div>
                <div style={{ fontSize: 10, color: COLORS.textMuted }}>TTL: {cache.ttl}</div>
              </div>
            ))}
          </div>

          <div style={{ fontSize: 14, fontWeight: 600, color: COLORS.textPrimary, marginBottom: 12 }}>缓存失效向量 (10种)</div>
          <div style={{ display: "flex", flexWrap: "wrap", gap: 4 }}>
            {[
              "SystemPromptChange", "ToolDefinitionChange", "NewMessage", "ToolResultChange",
              "ContextCompaction", "ModelSwitch", "SessionResume", "FileContentChange",
              "EnvVarChange", "UserPreferenceChange",
            ].map((v) => (
              <span key={v} style={{
                padding: "3px 8px", borderRadius: 4, fontSize: 10,
                fontFamily: "monospace", backgroundColor: COLORS.surfaceHover,
                color: COLORS.textSecondary, border: `1px solid ${COLORS.border}`,
              }}>{v}</span>
            ))}
          </div>
        </Card>
      </div>

      <Card>
        <div style={{ fontSize: 14, fontWeight: 600, color: COLORS.textPrimary, marginBottom: 16 }}>config.yaml</div>
        <div style={{
          padding: 16, borderRadius: 8, backgroundColor: COLORS.bg,
          border: `1px solid ${COLORS.border}`, fontFamily: "monospace",
          fontSize: 12, color: COLORS.textSecondary, lineHeight: 2,
        }}>
          <div><span style={{ color: COLORS.textMuted }}># 猎户座 Agent 配置</span></div>
          <div><span style={{ color: COLORS.textMuted }}># 所有 {"${VAR}"} 会被替换为环境变量</span></div>
          <div />
          <div><span style={{ color: COLORS.accent }}>provider:</span></div>
          <div style={{ paddingLeft: 16 }}><span style={{ color: COLORS.accentCyan }}>type:</span> <span style={{ color: COLORS.accentGreen }}>openai</span></div>
          <div style={{ paddingLeft: 16 }}><span style={{ color: COLORS.accentCyan }}>api_key:</span> <span style={{ color: COLORS.accentGreen }}>{"${LLM_API_KEY}"}</span></div>
          <div style={{ paddingLeft: 16 }}><span style={{ color: COLORS.accentCyan }}>api_base:</span> <span style={{ color: COLORS.accentGreen }}>{"${LLM_API_BASE}"}</span></div>
          <div style={{ paddingLeft: 16 }}><span style={{ color: COLORS.accentCyan }}>model:</span> <span style={{ color: COLORS.accentGreen }}>{"${LLM_MODEL}"}</span></div>
          <div style={{ paddingLeft: 16 }}><span style={{ color: COLORS.accentCyan }}>thinking:</span> <span style={{ color: COLORS.accentGreen }}>disabled</span></div>
          <div />
          <div><span style={{ color: COLORS.accent }}>cache:</span></div>
          <div style={{ paddingLeft: 16 }}><span style={{ color: COLORS.accentCyan }}>l1_max_entries:</span> <span style={{ color: COLORS.primary }}>2048</span></div>
          <div style={{ paddingLeft: 16 }}><span style={{ color: COLORS.accentCyan }}>l1_ttl_secs:</span> <span style={{ color: COLORS.primary }}>300</span></div>
          <div style={{ paddingLeft: 16 }}><span style={{ color: COLORS.accentCyan }}>l2_max_entries:</span> <span style={{ color: COLORS.primary }}>128</span></div>
          <div />
          <div><span style={{ color: COLORS.accent }}>orchestrator:</span></div>
          <div style={{ paddingLeft: 16 }}><span style={{ color: COLORS.accentCyan }}>mode:</span> <span style={{ color: COLORS.accentGreen }}>sequential</span></div>
          <div style={{ paddingLeft: 16 }}><span style={{ color: COLORS.accentCyan }}>max_workers:</span> <span style={{ color: COLORS.primary }}>3</span></div>
          <div style={{ paddingLeft: 16 }}><span style={{ color: COLORS.accentCyan }}>max_rounds:</span> <span style={{ color: COLORS.primary }}>10</span></div>
          <div />
          <div><span style={{ color: COLORS.accent }}>agent:</span></div>
          <div style={{ paddingLeft: 16 }}><span style={{ color: COLORS.accentCyan }}>max_turns:</span> <span style={{ color: COLORS.primary }}>50</span></div>
          <div style={{ paddingLeft: 16 }}><span style={{ color: COLORS.accentCyan }}>max_tool_calls:</span> <span style={{ color: COLORS.primary }}>30</span></div>
          <div style={{ paddingLeft: 16 }}><span style={{ color: COLORS.accentCyan }}>token_budget:</span> <span style={{ color: COLORS.primary }}>128000</span></div>
          <div />
          <div><span style={{ color: COLORS.accent }}>audit:</span></div>
          <div style={{ paddingLeft: 16 }}><span style={{ color: COLORS.accentCyan }}>enabled:</span> <span style={{ color: COLORS.primary }}>true</span></div>
          <div style={{ paddingLeft: 16 }}><span style={{ color: COLORS.accentCyan }}>path:</span> <span style={{ color: COLORS.accentGreen }}>./audit.log</span></div>
        </div>
      </Card>

      <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 16 }}>
        <Card>
          <div style={{ fontSize: 14, fontWeight: 600, color: COLORS.textPrimary, marginBottom: 16 }}>安全护栏配置</div>
          {[
            { name: "PermissionACL", desc: "路径级访问控制", enabled: true },
            { name: "TokenBudget", desc: "Token 用量上限 (128K)", enabled: true },
            { name: "BashRiskGrading", desc: "5 级命令风险分类", enabled: true },
            { name: "HookEngine", desc: "YAML 配置式拦截器", enabled: true },
            { name: "ExecPolicy", desc: "命令执行白名单", enabled: true },
            { name: "WorkspaceGuard", desc: "工作区边界保护", enabled: true },
          ].map((guard, i) => (
            <div key={i} style={{
              display: "flex", alignItems: "center", gap: 10, padding: "8px 0",
              borderBottom: i < 5 ? `1px solid ${COLORS.border}` : "none",
            }}>
              <div style={{
                width: 36, height: 20, borderRadius: 10,
                backgroundColor: guard.enabled ? COLORS.accentGreen : COLORS.border,
                position: "relative", cursor: "pointer",
              }}>
                <div style={{
                  width: 16, height: 16, borderRadius: "50%", backgroundColor: "#fff",
                  position: "absolute", top: 2,
                  left: guard.enabled ? 18 : 2, transition: "left 0.2s",
                }} />
              </div>
              <div style={{ flex: 1 }}>
                <div style={{ fontSize: 13, fontWeight: 600, color: COLORS.textPrimary }}>{guard.name}</div>
                <div style={{ fontSize: 11, color: COLORS.textMuted }}>{guard.desc}</div>
              </div>
            </div>
          ))}
        </Card>

        <Card>
          <div style={{ fontSize: 14, fontWeight: 600, color: COLORS.textPrimary, marginBottom: 16 }}>上下文压缩策略</div>
          {[
            { name: "Micro", desc: "最小压缩，保留完整上下文", threshold: "< 10 条消息", color: COLORS.accentGreen },
            { name: "Snip", desc: "截断旧工具结果，保留摘要", threshold: "10-20 条", color: COLORS.accentCyan },
            { name: "Chunked", desc: "分块压缩，按对话段切割", threshold: "20-30 条", color: COLORS.accent },
            { name: "Auto", desc: "自适应压缩，根据 Token 预算动态调整", threshold: "30-40 条", color: COLORS.accentPurple },
            { name: "Reactive", desc: "响应式压缩，接近上限时激进裁剪", threshold: "40-50 条", color: COLORS.accentRed },
            { name: "Collapse", desc: "极限压缩，只保留系统提示+最近几轮", threshold: "> 50 条", color: COLORS.accentRed },
          ].map((strat, i) => (
            <div key={i} style={{
              display: "flex", alignItems: "center", gap: 10, padding: "8px 0",
              borderBottom: i < 5 ? `1px solid ${COLORS.border}` : "none",
            }}>
              <div style={{
                width: 8, height: 8, borderRadius: 2, backgroundColor: strat.color,
              }} />
              <div style={{ width: 70, fontSize: 13, fontWeight: 700, color: strat.color }}>{strat.name}</div>
              <div style={{ flex: 1, fontSize: 12, color: COLORS.textSecondary }}>{strat.desc}</div>
              <Badge color={COLORS.textMuted}>{strat.threshold}</Badge>
            </div>
          ))}
        </Card>
      </div>
    </div>
  );
}

// ---- Main App ----
export default function OrionTUI() {
  const [activePage, setActivePage] = useState("dashboard");
  const [sidebarHover, setSidebarHover] = useState(null);

  const pages = {
    dashboard: DashboardPage,
    chat: ChatPage,
    orchestrator: OrchestratorPage,
    tools: ToolsPage,
    sessions: SessionsPage,
    audit: AuditPage,
    settings: SettingsPage,
  };
  const Page = pages[activePage];

  return (
    <div style={{
      display: "flex", height: "100vh", backgroundColor: COLORS.bg,
      fontFamily: '-apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif',
      color: COLORS.textPrimary, overflow: "hidden",
    }}>
      {/* Sidebar */}
      <div style={{
        width: 220, flexShrink: 0, backgroundColor: COLORS.surface,
        borderRight: `1px solid ${COLORS.border}`,
        display: "flex", flexDirection: "column",
      }}>
        {/* Logo */}
        <div style={{
          padding: "20px 16px", borderBottom: `1px solid ${COLORS.border}`,
          display: "flex", alignItems: "center", gap: 10,
        }}>
          <div style={{
            width: 36, height: 36, borderRadius: 10,
            background: `linear-gradient(135deg, ${COLORS.primary}, ${COLORS.accentPurple})`,
            display: "flex", alignItems: "center", justifyContent: "center",
          }}>
            <Star size={18} color="#fff" />
          </div>
          <div>
            <div style={{ fontSize: 15, fontWeight: 800, color: COLORS.textPrimary, letterSpacing: 0.5 }}>Orion</div>
            <div style={{ fontSize: 10, color: COLORS.textMuted }}>Agent Framework</div>
          </div>
        </div>

        {/* Navigation */}
        <div style={{ flex: 1, padding: "12px 8px", display: "flex", flexDirection: "column", gap: 2 }}>
          {navItems.map((item) => {
            const Icon = item.icon;
            const isActive = activePage === item.id;
            const isHover = sidebarHover === item.id;
            return (
              <button
                key={item.id}
                onClick={() => setActivePage(item.id)}
                onMouseEnter={() => setSidebarHover(item.id)}
                onMouseLeave={() => setSidebarHover(null)}
                style={{
                  display: "flex", alignItems: "center", gap: 10,
                  padding: "10px 12px", borderRadius: 8, border: "none",
                  cursor: "pointer", textAlign: "left", width: "100%",
                  backgroundColor: isActive ? COLORS.primary + "20" :
                    isHover ? COLORS.surfaceHover : "transparent",
                  color: isActive ? COLORS.primaryLight : COLORS.textSecondary,
                  transition: "all 0.15s",
                  fontSize: 13, fontWeight: isActive ? 600 : 400,
                }}
              >
                <Icon size={16} />
                {item.label}
                {isActive && (
                  <div style={{
                    marginLeft: "auto", width: 6, height: 6,
                    borderRadius: "50%", backgroundColor: COLORS.primaryLight,
                  }} />
                )}
              </button>
            );
          })}
        </div>

        {/* Footer */}
        <div style={{
          padding: "12px 16px", borderTop: `1px solid ${COLORS.border}`,
          fontSize: 11, color: COLORS.textMuted,
        }}>
          <div style={{ display: "flex", alignItems: "center", gap: 6 }}>
            <Radio size={10} color={COLORS.accentGreen} />
            <span>v0.1.0 · Rust 2021</span>
          </div>
          <div style={{ marginTop: 4, display: "flex", alignItems: "center", gap: 6 }}>
            <Circle size={6} fill={COLORS.accentGreen} color={COLORS.accentGreen} />
            <span>Tokio Runtime Active</span>
          </div>
        </div>
      </div>

      {/* Main Content */}
      <div style={{
        flex: 1, overflow: "hidden", display: "flex", flexDirection: "column",
      }}>
        {/* Top Bar */}
        <div style={{
          padding: "12px 24px", borderBottom: `1px solid ${COLORS.border}`,
          display: "flex", alignItems: "center", justifyContent: "space-between",
          backgroundColor: COLORS.surface,
        }}>
          <div style={{ display: "flex", alignItems: "center", gap: 12 }}>
            <span style={{ fontSize: 14, fontWeight: 600, color: COLORS.textPrimary }}>
              {navItems.find((n) => n.id === activePage)?.label}
            </span>
            <Badge color={COLORS.accentGreen}>运行中</Badge>
          </div>
          <div style={{ display: "flex", alignItems: "center", gap: 16 }}>
            <div style={{ display: "flex", alignItems: "center", gap: 6, fontSize: 12, color: COLORS.textMuted }}>
              <Cpu size={12} /> <span>deepseek-chat</span>
            </div>
            <div style={{ display: "flex", alignItems: "center", gap: 6, fontSize: 12, color: COLORS.textMuted }}>
              <Gauge size={12} /> <span>Token: 48.2K / 128K</span>
            </div>
            <div style={{
              width: 80, height: 4, borderRadius: 2, backgroundColor: COLORS.border, overflow: "hidden",
            }}>
              <div style={{ width: "37%", height: "100%", backgroundColor: COLORS.accentGreen, borderRadius: 2 }} />
            </div>
          </div>
        </div>

        {/* Page Content */}
        <div style={{
          flex: 1, overflowY: "auto", padding: 24,
        }}>
          <Page />
        </div>
      </div>
    </div>
  );
}
