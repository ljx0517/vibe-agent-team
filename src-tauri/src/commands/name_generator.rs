use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use tauri::command;

/// 英文人名结构体
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnglishName {
    pub zh: String,
    pub en: String,
    pub gender: String, // "男" 或 "女"
}

/// 获取所有英文名人名列表
pub fn get_all_english_names() -> Vec<EnglishName> {
    vec![
        // 男性名字
        EnglishName { zh: "奥利佛".to_string(), en: "Oliver".to_string(), gender: "男".to_string() },
        EnglishName { zh: "詹姆斯".to_string(), en: "James".to_string(), gender: "男".to_string() },
        EnglishName { zh: "威廉".to_string(), en: "William".to_string(), gender: "男".to_string() },
        EnglishName { zh: "本杰明".to_string(), en: "Benjamin".to_string(), gender: "男".to_string() },
        EnglishName { zh: "卢卡斯".to_string(), en: "Lucas".to_string(), gender: "男".to_string() },
        EnglishName { zh: "亨利".to_string(), en: "Henry".to_string(), gender: "男".to_string() },
        EnglishName { zh: "亚历山大".to_string(), en: "Alexander".to_string(), gender: "男".to_string() },
        EnglishName { zh: "伊森".to_string(), en: "Ethan".to_string(), gender: "男".to_string() },
        EnglishName { zh: "丹尼尔".to_string(), en: "Daniel".to_string(), gender: "男".to_string() },
        EnglishName { zh: "马修".to_string(), en: "Matthew".to_string(), gender: "男".to_string() },
        EnglishName { zh: "约瑟夫".to_string(), en: "Joseph".to_string(), gender: "男".to_string() },
        EnglishName { zh: "大卫".to_string(), en: "David".to_string(), gender: "男".to_string() },
        EnglishName { zh: "塞缪尔".to_string(), en: "Samuel".to_string(), gender: "男".to_string() },
        EnglishName { zh: "瑞安".to_string(), en: "Ryan".to_string(), gender: "男".to_string() },
        EnglishName { zh: "内森".to_string(), en: "Nathan".to_string(), gender: "男".to_string() },
        EnglishName { zh: "克里斯托弗".to_string(), en: "Christopher".to_string(), gender: "男".to_string() },
        EnglishName { zh: "安德鲁".to_string(), en: "Andrew".to_string(), gender: "男".to_string() },
        EnglishName { zh: "约书亚".to_string(), en: "Joshua".to_string(), gender: "男".to_string() },
        EnglishName { zh: "杰克".to_string(), en: "Jack".to_string(), gender: "男".to_string() },
        EnglishName { zh: "托马斯".to_string(), en: "Thomas".to_string(), gender: "男".to_string() },
        EnglishName { zh: "查尔斯".to_string(), en: "Charles".to_string(), gender: "男".to_string() },
        EnglishName { zh: "康纳".to_string(), en: "Connor".to_string(), gender: "男".to_string() },
        EnglishName { zh: "塞巴斯蒂安".to_string(), en: "Sebastian".to_string(), gender: "男".to_string() },
        EnglishName { zh: "亚当".to_string(), en: "Adam".to_string(), gender: "男".to_string() },
        EnglishName { zh: "朱利安".to_string(), en: "Julian".to_string(), gender: "男".to_string() },
        EnglishName { zh: "加布里埃尔".to_string(), en: "Gabriel".to_string(), gender: "男".to_string() },
        EnglishName { zh: "迪伦".to_string(), en: "Dylan".to_string(), gender: "男".to_string() },
        EnglishName { zh: "卢克".to_string(), en: "Luke".to_string(), gender: "男".to_string() },
        // 女性名字
        EnglishName { zh: "索菲亚".to_string(), en: "Sophia".to_string(), gender: "女".to_string() },
        EnglishName { zh: "艾玛".to_string(), en: "Emma".to_string(), gender: "女".to_string() },
        EnglishName { zh: "奥利维娅".to_string(), en: "Olivia".to_string(), gender: "女".to_string() },
        EnglishName { zh: "伊莎贝拉".to_string(), en: "Isabella".to_string(), gender: "女".to_string() },
        EnglishName { zh: "艾娃".to_string(), en: "Ava".to_string(), gender: "女".to_string() },
        EnglishName { zh: "米娅".to_string(), en: "Mia".to_string(), gender: "女".to_string() },
        EnglishName { zh: "夏洛特".to_string(), en: "Charlotte".to_string(), gender: "女".to_string() },
        EnglishName { zh: "阿米莉亚".to_string(), en: "Amelia".to_string(), gender: "女".to_string() },
        EnglishName { zh: "哈珀".to_string(), en: "Harper".to_string(), gender: "女".to_string() },
        EnglishName { zh: "伊芙琳".to_string(), en: "Evelyn".to_string(), gender: "女".to_string() },
        EnglishName { zh: "索菲".to_string(), en: "Sophie".to_string(), gender: "女".to_string() },
        EnglishName { zh: "格蕾丝".to_string(), en: "Grace".to_string(), gender: "女".to_string() },
        EnglishName { zh: "克洛伊".to_string(), en: "Chloe".to_string(), gender: "女".to_string() },
        EnglishName { zh: "维多利亚".to_string(), en: "Victoria".to_string(), gender: "女".to_string() },
        EnglishName { zh: "莱利".to_string(), en: "Riley".to_string(), gender: "女".to_string() },
        EnglishName { zh: "阿里亚".to_string(), en: "Aria".to_string(), gender: "女".to_string() },
        EnglishName { zh: "莉莉".to_string(), en: "Lily".to_string(), gender: "女".to_string() },
        EnglishName { zh: "奥罗拉".to_string(), en: "Aurora".to_string(), gender: "女".to_string() },
        EnglishName { zh: "佐伊".to_string(), en: "Zoey".to_string(), gender: "女".to_string() },
        EnglishName { zh: "佩内洛普".to_string(), en: "Penelope".to_string(), gender: "女".to_string() },
        EnglishName { zh: "莱拉".to_string(), en: "Layla".to_string(), gender: "女".to_string() },
        EnglishName { zh: "斯嘉丽".to_string(), en: "Scarlett".to_string(), gender: "女".to_string() },
        EnglishName { zh: "塞奇".to_string(), en: "Sage".to_string(), gender: "女".to_string() },
        EnglishName { zh: "维奥莱特".to_string(), en: "Violet".to_string(), gender: "女".to_string() },
        EnglishName { zh: "鲁比".to_string(), en: "Ruby".to_string(), gender: "女".to_string() },
        EnglishName { zh: "弗洛拉".to_string(), en: "Flora".to_string(), gender: "女".to_string() },
        EnglishName { zh: "珀尔".to_string(), en: "Pearl".to_string(), gender: "女".to_string() },
        EnglishName { zh: "艾瑞斯".to_string(), en: "Iris".to_string(), gender: "女".to_string() },
        EnglishName { zh: "杰德".to_string(), en: "Jade".to_string(), gender: "女".to_string() },
        EnglishName { zh: "锡达".to_string(), en: "Cedar".to_string(), gender: "女".to_string() },
    ]
}

/// 随机获取一个英文名
/// # Arguments
/// * `gender` - 可选参数，指定性别 ("男" 或 "女")，不指定则随机
pub fn random_english_name(gender: Option<&str>) -> EnglishName {
    let names = get_all_english_names();

    let filtered: Vec<EnglishName> = match gender {
        Some(g) if g == "男" => names.into_iter().filter(|n| n.gender == "男").collect(),
        Some(g) if g == "女" => names.into_iter().filter(|n| n.gender == "女").collect(),
        _ => names,
    };

    use std::time::{SystemTime, UNIX_EPOCH};
    let seed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos() as usize;

    let index = seed % filtered.len();
    filtered[index].clone()
}

/// 生成随机颜色
pub fn get_random_color() -> String {
    let colors = vec![
        "#FF6B6B", "#4ECDC4", "#45B7D1", "#96CEB4", "#FFEAA7",
        "#DDA0DD", "#98D8C8", "#F7DC6F", "#BB8FCE", "#85C1E9",
    ];

    use std::time::{SystemTime, UNIX_EPOCH};
    let seed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos() as usize;

    let index = seed % colors.len();
    colors[index].to_string()
}

/// 生成合法的文件夹名
/// # Arguments
/// * `name` - 原始名称
pub fn to_legal_folder_name(name: &str) -> String {
    let mut result = name.to_lowercase();

    // 空格替换为 -
    result = result.replace(' ', "-");

    // 移除非法字符（包括 !）
    let illegal_chars = ['/', ':', '?', '*', '"', '<', '>', '|', '!'];
    for c in illegal_chars {
        result = result.replace(c, "");
    }

    // 连续短横线合并为一个
    while result.contains("--") {
        result = result.replace("--", "-");
    }

    // 移除中文字符（简单处理：只保留 ASCII 字符）
    result = result.chars().filter(|c| c.is_ascii()).collect();

    // 移除首尾连字符
    result = result.trim_matches('-').to_string();

    result
}

/// 导出人名列表到 JSON 文件（可选功能）
#[allow(dead_code)]
pub fn export_names_to_json(path: &PathBuf) -> Result<(), String> {
    let names = get_all_english_names();
    let json = serde_json::to_string_pretty(&names)
        .map_err(|e| format!("JSON 序列化失败: {}", e))?;

    fs::write(path, json)
        .map_err(|e| format!("文件写入失败: {}", e))?;

    Ok(())
}

// ============ Tauri Commands ============

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RandomNameRequest {
    pub gender: Option<String>, // "男" 或 "女"，不传则随机
}

#[command]
pub fn cmd_random_english_name(gender: Option<String>) -> EnglishName {
    random_english_name(gender.as_deref())
}

#[command]
pub fn cmd_get_random_color() -> String {
    get_random_color()
}

#[command]
pub fn cmd_to_legal_folder_name(name: String) -> String {
    to_legal_folder_name(&name)
}

#[command]
pub fn cmd_get_all_english_names() -> Vec<EnglishName> {
    get_all_english_names()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_random_english_name() {
        let name = random_english_name(None);
        assert!(!name.en.is_empty());
        assert!(!name.zh.is_empty());
    }

    #[test]
    fn test_random_english_name_with_gender() {
        let male_name = random_english_name(Some("男"));
        assert_eq!(male_name.gender, "男");

        let female_name = random_english_name(Some("女"));
        assert_eq!(female_name.gender, "女");
    }

    #[test]
    fn test_to_legal_folder_name() {
        assert_eq!(to_legal_folder_name("My Project 123!"), "my-project-123");
        assert_eq!(to_legal_folder_name("AI Agent 🤖"), "ai-agent");
    }

    #[test]
    fn test_get_random_color() {
        let color = get_random_color();
        assert!(color.starts_with('#'));
    }
}
