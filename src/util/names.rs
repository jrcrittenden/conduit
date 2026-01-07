//! Workspace name generation using adjective-noun combinations

use rand::prelude::IndexedRandom;

/// Short adjectives (max 4 chars) for workspace names
const ADJECTIVES: &[&str] = &[
    "bold", "calm", "cool", "dark", "deep", "fair", "fast", "free", "glad", "gold", "keen", "kind",
    "live", "lone", "lost", "loud", "mild", "near", "neat", "new", "nice", "old", "pale", "pure",
    "rare", "raw", "red", "rich", "safe", "shy", "slim", "slow", "soft", "tall", "tame", "thin",
    "tiny", "trim", "true", "vast", "warm", "weak", "wide", "wild", "wise",
];

/// Short nouns (max 4 chars) for workspace names
const NOUNS: &[&str] = &[
    "dune", "fern", "fox", "hawk", "hill", "iris", "jade", "lake", "lark", "leaf", "lynx", "mesa",
    "mist", "moon", "moss", "oak", "owl", "peak", "pine", "pond", "rain", "reef", "rock", "sage",
    "seal", "snow", "star", "sun", "swan", "tide", "vale", "wave", "wind", "wolf", "wren",
];

/// Generate a unique workspace name not in the existing list
///
/// Uses adjective-noun combinations (e.g., "bold-fox", "calm-owl").
/// With 45 adjectives and 35 nouns, there are 1,575 unique combinations.
/// If all combinations are exhausted, falls back to UUID suffix.
pub fn generate_workspace_name(existing: &[String]) -> String {
    let mut rng = rand::rng();

    // Try random combinations until we find an unused one
    // With 1,575 combinations, 100 attempts should be plenty
    for _ in 0..100 {
        let adj = ADJECTIVES.choose(&mut rng).unwrap_or(&"bold");
        let noun = NOUNS.choose(&mut rng).unwrap_or(&"fox");
        let candidate = format!("{}-{}", adj, noun);

        if !existing.contains(&candidate) {
            return candidate;
        }
    }

    // Fallback: add short UUID suffix (extremely unlikely to reach this)
    let adj = ADJECTIVES.choose(&mut rng).unwrap_or(&"bold");
    let noun = NOUNS.choose(&mut rng).unwrap_or(&"fox");
    let uuid_suffix = &uuid::Uuid::new_v4().as_simple().to_string()[..4];
    format!("{}-{}-{}", adj, noun, uuid_suffix)
}

/// Generate a branch name from username and workspace name
///
/// Format: `username/workspace-name`
pub fn generate_branch_name(username: &str, workspace_name: &str) -> String {
    let sanitized_username = sanitize_git_ref(username);
    format!("{}/{}", sanitized_username, workspace_name)
}

/// Get the username for branch naming
///
/// Priority:
/// 1. OS username (USER or USERNAME environment variable)
/// 2. git config user.name
/// 3. Fallback to "user"
pub fn get_git_username() -> String {
    // Try OS username first (USER on Unix, USERNAME on Windows)
    if let Ok(user) = std::env::var("USER") {
        if !user.is_empty() {
            return sanitize_git_ref(&user);
        }
    }

    if let Ok(user) = std::env::var("USERNAME") {
        if !user.is_empty() {
            return sanitize_git_ref(&user);
        }
    }

    // Fall back to git config
    if let Ok(output) = std::process::Command::new("git")
        .args(["config", "user.name"])
        .output()
    {
        if output.status.success() {
            let name = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !name.is_empty() {
                return sanitize_git_ref(&name);
            }
        }
    }

    "user".to_string()
}

/// Sanitize a string for use in git refs
///
/// - Lowercase
/// - Replace spaces with hyphens
/// - Remove characters not allowed in git refs
fn sanitize_git_ref(input: &str) -> String {
    input
        .to_lowercase()
        .chars()
        .map(|c| match c {
            ' ' | '_' => '-',
            c if c.is_alphanumeric() || c == '-' || c == '.' => c,
            _ => '-',
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_workspace_name_empty_existing() {
        let name = generate_workspace_name(&[]);
        // Should be adjective-noun format
        let parts: Vec<&str> = name.split('-').collect();
        assert_eq!(parts.len(), 2);
        assert!(ADJECTIVES.contains(&parts[0]));
        assert!(NOUNS.contains(&parts[1]));
    }

    #[test]
    fn test_generate_workspace_name_avoids_existing() {
        let existing = vec!["bold-fox".to_string(), "calm-owl".to_string()];
        let name = generate_workspace_name(&existing);
        assert!(!existing.contains(&name));
    }

    #[test]
    fn test_generate_workspace_name_format() {
        let name = generate_workspace_name(&[]);
        // Verify format is adjective-noun
        assert!(name.contains('-'));
        let parts: Vec<&str> = name.split('-').collect();
        assert!(parts.len() >= 2);
        // First part should be an adjective
        assert!(ADJECTIVES.contains(&parts[0]));
    }

    #[test]
    fn test_generate_branch_name() {
        let branch = generate_branch_name("fcoury", "bold-fox");
        assert_eq!(branch, "fcoury/bold-fox");
    }

    #[test]
    fn test_generate_branch_name_sanitizes() {
        let branch = generate_branch_name("Felipe Coury", "calm-owl");
        assert_eq!(branch, "felipe-coury/calm-owl");
    }

    #[test]
    fn test_sanitize_git_ref() {
        assert_eq!(sanitize_git_ref("Hello World"), "hello-world");
        assert_eq!(sanitize_git_ref("user_name"), "user-name");
        assert_eq!(sanitize_git_ref("John.Doe"), "john.doe");
        assert_eq!(sanitize_git_ref("--test--"), "test");
    }

    #[test]
    fn test_total_combinations() {
        // Verify we have enough combinations (should be 1,575)
        let total = ADJECTIVES.len() * NOUNS.len();
        assert_eq!(total, 45 * 35);
        assert!(
            total > 1500,
            "Should have at least 1500 unique combinations"
        );
    }
}
