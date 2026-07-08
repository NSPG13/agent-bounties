use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;

#[derive(Debug, Deserialize)]
pub struct DiscoveryComment {
    pub author: String,
    pub body: String,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct ContributorRecord {
    pub author: String,
    pub discovery_sources: Vec<String>,
    pub participation_reasons: Vec<String>,
    pub useful_labels: Vec<String>,
    pub trust_payment_signals: Vec<String>,
    pub friction_points: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct DiscoverySummary {
    pub contributors: Vec<ContributorRecord>,
}

pub fn discovery_report(
    input_fixture: String,
    json_out: String,
    markdown_out: String,
) -> Result<()> {
    let input_content = fs::read_to_string(input_fixture)?;
    let comments: Vec<DiscoveryComment> = serde_json::from_str(&input_content)?;

    let mut contributors: HashMap<String, ContributorRecord> = HashMap::new();

    for comment in comments {
        let record = contributors
            .entry(comment.author.clone())
            .or_insert_with(|| ContributorRecord {
                author: comment.author.clone(),
                ..Default::default()
            });

        let lines: Vec<&str> = comment.body.lines().collect();
        let mut current_section = "";

        for line in lines {
            let lower_line = line.to_lowercase();
            if lower_line.contains("how did you find this bounty") {
                current_section = "discovery";
            } else if lower_line.contains("what made it worth participating in") {
                current_section = "participation";
            } else if !line.trim().is_empty() && !line.starts_with("1.") && !line.starts_with("2.")
            {
                match current_section {
                    "discovery" => record.discovery_sources.push(line.trim().to_string()),
                    "participation" => record.participation_reasons.push(line.trim().to_string()),
                    _ => {}
                }

                if lower_line.contains("friction") {
                    record.friction_points.push(line.trim().to_string());
                }
                if lower_line.contains("trust")
                    || lower_line.contains("payment")
                    || lower_line.contains("usdc")
                {
                    record.trust_payment_signals.push(line.trim().to_string());
                }
                if lower_line.contains("label") {
                    record.useful_labels.push(line.trim().to_string());
                }
            }
        }
    }

    let mut summary = DiscoverySummary::default();
    for (_, record) in contributors {
        summary.contributors.push(record);
    }
    summary.contributors.sort_by(|a, b| a.author.cmp(&b.author));

    let json_content = serde_json::to_string_pretty(&summary)?;
    if let Some(parent) = std::path::Path::new(&json_out).parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&json_out, json_content)?;

    let mut md_content = String::new();
    md_content.push_str("# Contributor Discovery Attribution Report\n\n");

    for record in &summary.contributors {
        md_content.push_str(&format!("## Contributor: {}\n\n", record.author));

        if !record.discovery_sources.is_empty() {
            md_content.push_str("### Discovery Sources\n");
            for item in &record.discovery_sources {
                md_content.push_str(&format!("- {}\n", item));
            }
            md_content.push_str("\n");
        }

        if !record.participation_reasons.is_empty() {
            md_content.push_str("### Participation Reasons\n");
            for item in &record.participation_reasons {
                md_content.push_str(&format!("- {}\n", item));
            }
            md_content.push_str("\n");
        }

        if !record.useful_labels.is_empty() {
            md_content.push_str("### Useful Labels\n");
            for item in &record.useful_labels {
                md_content.push_str(&format!("- {}\n", item));
            }
            md_content.push_str("\n");
        }

        if !record.trust_payment_signals.is_empty() {
            md_content.push_str("### Trust & Payment Signals\n");
            for item in &record.trust_payment_signals {
                md_content.push_str(&format!("- {}\n", item));
            }
            md_content.push_str("\n");
        }

        if !record.friction_points.is_empty() {
            md_content.push_str("### Friction Points\n");
            for item in &record.friction_points {
                md_content.push_str(&format!("- {}\n", item));
            }
            md_content.push_str("\n");
        }
    }

    if let Some(parent) = std::path::Path::new(&markdown_out).parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&markdown_out, md_content)?;

    println!(
        "Report generated. JSON: {}, Markdown: {}",
        json_out, markdown_out
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;
    use std::fs;

    fn get_unique_temp_dir() -> std::path::PathBuf {
        let mut dir = env::temp_dir();
        dir.push(uuid::Uuid::new_v4().to_string());
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn test_discovery_report_handles_missing_and_partial_answers() {
        let dir = get_unique_temp_dir();
        let input_path = dir.join("input.json");
        let json_out = dir.join("out.json");
        let md_out = dir.join("out.md");

        let input_data = r#"[
            {
                "author": "user1",
                "body": "1. How did you find this bounty?\nFound it on twitter."
            },
            {
                "author": "user2",
                "body": "2. What made it worth participating in?\nUSDC payment."
            }
        ]"#;
        fs::write(&input_path, input_data).unwrap();

        let result = discovery_report(
            input_path.to_str().unwrap().to_string(),
            json_out.to_str().unwrap().to_string(),
            md_out.to_str().unwrap().to_string(),
        );

        assert!(result.is_ok());

        let out_json: DiscoverySummary =
            serde_json::from_str(&fs::read_to_string(json_out).unwrap()).unwrap();
        assert_eq!(out_json.contributors.len(), 2);

        let user1 = out_json
            .contributors
            .iter()
            .find(|c| c.author == "user1")
            .unwrap();
        assert_eq!(user1.discovery_sources.len(), 1);

        let user2 = out_json
            .contributors
            .iter()
            .find(|c| c.author == "user2")
            .unwrap();
        assert_eq!(user2.participation_reasons.len(), 1);
        assert_eq!(user2.trust_payment_signals.len(), 1);

        assert!(md_out.exists());
    }

    #[test]
    fn test_discovery_report_handles_duplicate_contributors_and_noisy_threads() {
        let dir = get_unique_temp_dir();
        let input_path = dir.join("input.json");
        let json_out = dir.join("out.json");
        let md_out = dir.join("out.md");

        let input_data = r#"[
            {
                "author": "noisy1",
                "body": "Hey guys, I have a question about this bounty. Not related to discovery."
            },
            {
                "author": "user1",
                "body": "1. How did you find this bounty?\nGitHub label.\n2. What made it worth participating in?\nI like the project."
            },
            {
                "author": "user1",
                "body": "Oh wait, I forgot to add: friction was high."
            }
        ]"#;
        fs::write(&input_path, input_data).unwrap();

        let result = discovery_report(
            input_path.to_str().unwrap().to_string(),
            json_out.to_str().unwrap().to_string(),
            md_out.to_str().unwrap().to_string(),
        );

        assert!(result.is_ok());
        let out_json: DiscoverySummary =
            serde_json::from_str(&fs::read_to_string(json_out).unwrap()).unwrap();

        // user1 and noisy1
        assert_eq!(out_json.contributors.len(), 2);

        let user1 = out_json
            .contributors
            .iter()
            .find(|c| c.author == "user1")
            .unwrap();
        assert_eq!(user1.discovery_sources.len(), 1);
        assert_eq!(user1.useful_labels.len(), 1); // Because it contains "label"
        assert_eq!(user1.participation_reasons.len(), 1);
        assert_eq!(user1.friction_points.len(), 1);
    }

    #[test]
    fn test_discovery_report_creates_parent_directories() {
        let dir = get_unique_temp_dir();
        let input_path = dir.join("input.json");
        let nested_dir = dir.join("deep").join("nested");
        let json_out = nested_dir.join("out.json");
        let md_out = nested_dir.join("out.md");

        let input_data = r#"[
            {
                "author": "user1",
                "body": "1. How did you find this bounty?\nFound it on twitter."
            }
        ]"#;
        fs::write(&input_path, input_data).unwrap();

        let result = discovery_report(
            input_path.to_str().unwrap().to_string(),
            json_out.to_str().unwrap().to_string(),
            md_out.to_str().unwrap().to_string(),
        );

        assert!(result.is_ok());
        assert!(json_out.exists());
        assert!(md_out.exists());
    }
}
