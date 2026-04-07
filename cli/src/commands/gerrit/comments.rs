// Copyright 2024 The Jujutsu Authors
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
// https://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use std::collections::HashMap;
use std::io::Write as _;

use serde::Deserialize;
use serde::Serialize;

use super::query::QueryArgs;
use crate::cli_util::CommandHelper;
use crate::command_error::CommandError;
use crate::ui::Ui;

/// Show Gerrit comments for a change.
#[derive(clap::Args, Clone, Debug)]
pub struct CommentsArgs {
    #[command(flatten)]
    query: QueryArgs,

    /// Include resolved comment threads.
    #[arg(long, default_value_t = false)]
    pub resolved: bool,
}

// This is the format returned by the Gerrit API.
// See https://gerrit-review.googlesource.com/Documentation/rest-api-changes.html#list-change-comments
// It is, however, not structured in a manner useful to the user.
type CommentsResponse = HashMap<String, Vec<CommentInfo>>;

#[derive(Deserialize, Serialize, Debug)]
// Note: This is incomplete, but contains all the fields we care about.
// If you want more fields, feel free to add them.
struct CommentInfo {
    pub author: Author,
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub in_reply_to: Option<String>,
    pub message: String,
    pub unresolved: bool,
    pub patch_set: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
}

#[derive(Deserialize, Serialize, Debug)]
struct Author {
    pub name: Option<String>,
    pub email: Option<String>,
}

#[derive(Debug, Serialize, Eq, PartialEq)]
struct CommentThread {
    pub file: String,
    pub line: Option<u32>,
    pub comments: Vec<SimpleComment>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub resolved: bool,
}

// This is a simplified version of a comment.
// It is part of a
#[derive(Debug, Serialize, Eq, PartialEq)]
struct SimpleComment {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    pub message: String,
}

fn simplify_response(comments: &CommentsResponse, args: &CommentsArgs) -> Vec<CommentThread> {
    let comment_ids: HashMap<String, &CommentInfo> = comments
        .values()
        .flat_map(|v| v.iter().map(|c| (c.id.clone(), c)))
        .collect();
    let replied_to: std::collections::HashSet<&str> = comments
        .values()
        .flat_map(|v| v.iter().filter_map(|c| c.in_reply_to.as_deref()))
        .collect();

    let mut threads = Vec::new();
    for (file, comments_list) in comments {
        for comment in comments_list {
            // A leaf comment is one that NO other comment replies to
            if !replied_to.contains(comment.id.as_str()) && (args.resolved || comment.unresolved) {
                let mut cur = Some(comment);
                let mut thread = CommentThread {
                    file: file.clone(),
                    line: comment.line,
                    resolved: !comment.unresolved,
                    comments: Vec::new(),
                };

                while let Some(c) = cur {
                    thread.comments.push(SimpleComment {
                        author: match &c.author {
                            Author {
                                name: Some(name),
                                email: Some(email),
                            } => Some(format!("{name} <{email}>")),
                            Author {
                                name: Some(name),
                                email: None,
                            } => Some(name.clone()),
                            Author {
                                name: None,
                                email: Some(email),
                            } => Some(email.clone()),
                            Author {
                                name: None,
                                email: None,
                            } => None,
                        },
                        message: c.message.clone(),
                    });
                    thread.line = thread.line.or(c.line);
                    cur = c
                        .in_reply_to
                        .as_ref()
                        .and_then(|id| comment_ids.get(id).copied());
                }
                thread.comments.reverse();
                threads.push(thread);
            }
        }
    }
    threads
}

pub async fn cmd_gerrit_comments(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &CommentsArgs,
) -> Result<(), CommandError> {
    let comments: CommentsResponse = args.query.query(ui, command, "changes", "comments").await?;
    let simplified = simplify_response(&comments, args);
    let json_str = serde_json::to_string_pretty(&simplified).map_err(|e| {
        crate::command_error::user_error(format!("Failed to serialize comments to JSON: {e}"))
    })?;
    writeln!(ui.stdout(), "{json_str}")
        .map_err(|e| crate::command_error::user_error(e.to_string()))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simplify_response() {
        let mut comments = HashMap::new();
        comments.insert(
            "src/main.rs".to_string(),
            vec![
                CommentInfo {
                    author: Author {
                        name: Some("Alice".to_string()),
                        email: Some("alice@example.com".to_string()),
                    },
                    id: "c1".to_string(),
                    in_reply_to: None,
                    message: "Resolved comment".to_string(),
                    unresolved: true,
                    patch_set: 1,
                    line: Some(42),
                },
                CommentInfo {
                    author: Author {
                        name: Some("Bob".to_string()),
                        email: None,
                    },
                    id: "c2".to_string(),
                    in_reply_to: Some("c1".to_string()),
                    message: "Resolved response".to_string(),
                    unresolved: false,
                    patch_set: 1,
                    line: Some(50),
                },
                CommentInfo {
                    author: Author {
                        name: None,
                        email: Some("alice@example.com".to_string()),
                    },
                    id: "c3".to_string(),
                    in_reply_to: None,
                    message: "Unresolved comment".to_string(),
                    unresolved: false,
                    patch_set: 1,
                    line: Some(42),
                },
                CommentInfo {
                    author: Author {
                        name: None,
                        email: None,
                    },
                    id: "c4".to_string(),
                    in_reply_to: Some("c3".to_string()),
                    message: "Unresolved response".to_string(),
                    unresolved: true,
                    patch_set: 1,
                    line: Some(50),
                },
            ],
        );

        let query = QueryArgs {
            revision: crate::cli_util::RevisionArg::from("@".to_string()),
        };

        let all_threads = simplify_response(&comments, &CommentsArgs { query: query.clone(), resolved: true });
        assert_eq!(
            all_threads.len(),
            2,
            "Expected both resolved and unresolved threads to be included"
        );
        assert_eq!(
            all_threads,
            vec![
                CommentThread {
                    file: "src/main.rs".to_string(),
                    line: Some(50),
                    resolved: true,
                    comments: vec![
                        SimpleComment {
                            author: Some("Alice <alice@example.com>".to_string()),
                            message: "Resolved comment".to_string(),
                        },
                        SimpleComment {
                            author: Some("Bob".to_string()),
                            message: "Resolved response".to_string(),
                        },
                    ],
                },
                CommentThread {
                    file: "src/main.rs".to_string(),
                    line: Some(50),
                    resolved: false,
                    comments: vec![
                        SimpleComment {
                            author: Some("alice@example.com".to_string()),
                            message: "Unresolved comment".to_string(),
                        },
                        SimpleComment {
                            author: None,
                            message: "Unresolved response".to_string(),
                        },
                    ],
                },
            ]
        );

        let unresolved_threads = simplify_response(&comments, &CommentsArgs { query, resolved: false });
        assert_eq!(
            unresolved_threads.len(),
            1,
            "Expected only the unresolved thread to be included when resolved=false"
        );
        assert_eq!(unresolved_threads[0].resolved, false);
        assert!(
            all_threads.contains(&unresolved_threads[0]),
            "Expected the unresolved thread to exist in all_threads"
        );
    }
}
