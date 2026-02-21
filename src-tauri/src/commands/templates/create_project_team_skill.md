---
name: create-project-team
description: 为项目创建开发团队，生成 Team Lead 和 Reviewer（Devil's Advocate）信息，创建团队配置文件
argument-hint: <project-name> <project-description> <workspace-path>
disable-model-invocation: true
---

# Create Project Team

为项目创建开发团队，生成 Team Lead 和 Reviewer 成员信息。

## 输入参数

- `$0` = project-name（项目名称）
- `$1` = project-description（项目描述）
- `$2` = workspace-path（工作目录路径）

## 执行步骤

### 1. 随机生成两个人名

从以下列表中随机选择 2 个英文名，确保性别不同（一人男，一人女）：

**男性英文名：**
- Oliver, James, William, Benjamin, Lucas, Henry, Alexander, Ethan, Daniel, Matthew
- Henry, Joseph, David, Samuel, Ryan, Nathan, Christopher, Andrew, Joshua, Benjamin
- Jack, Thomas, Charles, Connor, Sebastian, Adam, Julian, Gabriel, Dylan, Luke

**女性英文名：**
- Sophia, Emma, Olivia, Isabella, Ava, Mia, Charlotte, Amelia, Harper, Evelyn
- Sophie, Grace, Chloe, Victoria, Riley, Aria, Lily, Aurora, Zoey, Penelope
- Layla, Scarlett, Sage, Violet, Ruby, Flora, Pearl, Iris, Jade, Cedar

### 2. 翻译成中文名（5字以内）

翻译规则：
- 男性常见中文名：奥利弗、詹姆斯、威廉、卢卡斯、亨利、亚历山大、伊桑、丹尼尔、马修、约瑟夫、大卫、塞缪尔、瑞安、克里斯托弗、安德鲁、乔舒亚、杰克、托马斯、查尔斯、塞巴斯蒂安
- 女性常见中文名：苏菲、艾玛、奥利维亚、伊莎贝拉、艾娃、米娅、夏洛特、艾米丽、伊芙琳、格雷丝、克洛伊、维多利亚、莱莉、艾莉娅、莉莉、紫罗兰、露比、弗洛拉

### 3. 生成 Reviewer Prompt

为 reviewer 生成 devil's advocate 角色的 prompt：

```markdown
你是 {{reviewer_name}}，项目 {{project_name}} 的资深技术评审专家（Devil's Advocate）。

## 角色背景
- 20年以上IT行业经验
- 精通需求分析、系统架构、设计模式、编码规范
- 熟悉从立项到运维的全生命周期
- 擅长发现问题、提出质疑、推动改进
- 严格审查技术方案，确保质量和可行性

## 评审原则
1. 质疑一切不合理的假设
2. 挑战模糊或不完整的需求
3. 检查方案的扩展性和维护性
4. 确保安全性和性能考量
5. 验证测试覆盖的完整性

## 沟通风格
- 理性、直接、客观
- 用数据和事实支持观点
- 提供建设性的替代方案

当团队讨论技术方案时，你必须：
- 指出潜在风险和漏洞
- 提问挑战现有假设
- 要求澄清模糊点
- 推荐更好的替代方案
```

### 4. 生成 team-name（合法文件夹名）

将项目名转换为合法文件夹名：
- 转小写
- 空格替换为 `-`
- 移除非法字符（`/:?*"<>|`）
- 连续短横线合并为一个
- 不能有中文字符

示例：
- "My Project 123!" → `my-project-123`
- "AI Agent 🤖" → `ai-agent`

### 5. 生成随机颜色

从以下颜色中随机选择一个：
- `#FF6B6B`, `#4ECDC4`, `#45B7D1`, `#96CEB4`, `#FFEAA7`, `#DDA0DD`, `#98D8C8`, `#F7DC6F`, `#BB8FCE`, `#85C1E9`

### 6. 创建团队配置文件

获取当前时间戳（毫秒）：

```bash
date +%s000
```

创建目录并写入 config.json：

```bash
mkdir -p ~/.claude/teams/{team-name}
mkdir -p ~/.claude/tasks/{team-name}
```

config.json 内容：

```json
{
  "name": "{{project_name}}",
  "description": "{{project_description}}｜{{project_name}}项目开发团队 - Team Lead {{leader_name}}",
  "createdAt": {{current_timestamp}},
  "leadAgentId": "{{leader_en_name}}@{{project_name}}",
  "leadSessionId": "{{uuid}}",
  "members": [
    {
      "agentId": "{{leader_en_name}}@{{project_name}}",
      "name": "{{leader_en_name}}",
      "agentType": "{{leader_en_name}}",
      "model": "",
      "joinedAt": {{current_timestamp}},
      "tmuxPaneId": "",
      "cwd": "{{workspace_path}}",
      "subscriptions": []
    },
    {
      "agentId": "{{reviewer_en_name}}@{{project_name}}",
      "name": "{{reviewer_en_name}}",
      "agentType": "general-purpose",
      "model": "",
      "prompt": "{{reviewer_prompt}}",
      "color": "{{random_color}}",
      "planModeRequired": false,
      "joinedAt": {{current_timestamp}},
      "tmuxPaneId": "",
      "cwd": "{{workspace_path}}",
      "subscriptions": [],
      "backendType": "auto"
    }
  ]
}
```

## 输出格式

然后输出已创建的 config.json 完整内容（确保输出是有效 JSON 格式，不需要其他内容）。

## 注意事项

- team-name 必须是合法的文件夹名称
- 确保 JSON 格式正确（无尾随逗号）
- 使用当前时间戳
- workspace-path 使用调用时传入的实际路径
