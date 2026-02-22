---
name: create-project-team
description: 为项目创建开发团队，生成 Team Lead 和 Reviewer（Devil's Advocate）信息，创建团队配置文件
argument-hint: <project-name> <project-description> <workspace-path>
disable-model-invocation: true
---

# Create Project Team

为项目创建开发团队，生成 Team Lead 成员信息。

## 输入参数

- `$0` = project-name（项目名称）
- `$1` = project-description（项目描述）
- `$2` = workspace-path（工作目录路径）

## 执行步骤

### 1. 随机生成一个人名

从以下列表中随机选择 1 个人物作为Team Lead。

| 中文 | 英文 | 性别 |
| --- | --- | --- |
| 奥利佛 | Oliver | 男 |
| 詹姆斯 | James | 男 |
| 威廉 | William | 男 |
| 本杰明 | Benjamin | 男 |
| 卢卡斯 | Lucas | 男 |
| 亨利 | Henry | 男 |
| 亚历山大 | Alexander | 男 |
| 伊森 | Ethan | 男 |
| 丹尼尔 | Daniel | 男 |
| 马修 | Matthew | 男 |
| 约瑟夫 | Joseph | 男 |
| 大卫 | David | 男 |
| 塞缪尔 | Samuel | 男 |
| 瑞安 | Ryan | 男 |
| 内森 | Nathan | 男 |
| 克里斯托弗 | Christopher | 男 |
| 安德鲁 | Andrew | 男 |
| 约书亚 | Joshua | 男 |
| 杰克 | Jack | 男 |
| 托马斯 | Thomas | 男 |
| 查尔斯 | Charles | 男 |
| 康纳 | Connor | 男 |
| 塞巴斯蒂安 | Sebastian | 男 |
| 亚当 | Adam | 男 |
| 朱利安 | Julian | 男 |
| 加布里埃尔 | Gabriel | 男 |
| 迪伦 | Dylan | 男 |
| 卢克 | Luke | 男 |
| 索菲亚 | Sophia | 女 |
| 艾玛 | Emma | 女 |
| 奥利维娅 | Olivia | 女 |
| 伊莎贝拉 | Isabella | 女 |
| 艾娃 | Ava | 女 |
| 米娅 | Mia | 女 |
| 夏洛特 | Charlotte | 女 |
| 阿米莉亚 | Amelia | 女 |
| 哈珀 | Harper | 女 |
| 伊芙琳 | Evelyn | 女 |
| 索菲 | Sophie | 女 |
| 格蕾丝 | Grace | 女 |
| 克洛伊 | Chloe | 女 |
| 维多利亚 | Victoria | 女 |
| 莱利 | Riley | 女 |
| 阿里亚 | Aria | 女 |
| 莉莉 | Lily | 女 |
| 奥罗拉 | Aurora | 女 |
| 佐伊 | Zoey | 女 |
| 佩内洛普 | Penelope | 女 |
| 莱拉 | Layla | 女 |
| 斯嘉丽 | Scarlett | 女 |
| 塞奇 | Sage | 女 |
| 维奥莱特 | Violet | 女 |
| 鲁比 | Ruby | 女 |
| 弗洛拉 | Flora | 女 |
| 珀尔 | Pearl | 女 |
| 艾瑞斯 | Iris | 女 |
| 杰德 | Jade | 女 |
| 锡达 | Cedar | 女 |


### 3. 生成 Team Lead Prompt

为 Team Lead 角色生成 prompt： 角色是Team Lead，能力包含必须是是Software Architect ，并且devil's advocate， 还有丰富的产品思维，和经验
** 备注 ** : 如果是女性角色，偏产品一些，如果是男性角色偏技术一些


### 4. 生成 team-name（合法文件夹名）

将项目名转换为合法文件夹名：
- 转小写
- 空格替换为 `-`
- 移除非法字符（`/:?*"<>|`）
- 连续短横线合并为一个
- 不能有中文字符（可以把中文变英文，或者转拼音）

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
      "nickname": "{{leader_zh_name}}",
      "gender":"{{leader_gender}}",
      "agentType": "general-purpose",
      "model": "",
      "prompt": "{{lead_prompt}}",
      "joinedAt": {{current_timestamp}},
      "tmuxPaneId": "",
      "cwd": "{{workspace_path}}",
      "subscriptions": []
    }
  ]
}
```

## 输出格式

然后输出已创建的 config.json 完整内容（确保是有效 JSON 格式）。

## 注意事项

- team-name 必须是合法的文件夹名称
- 确保 JSON 格式正确（无尾随逗号）
- 使用当前时间戳
- workspace-path 使用调用时传入的实际路径