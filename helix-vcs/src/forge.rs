pub fn commit_url(remote: &str, commit: &str) -> Option<String> {
    let base = web_base_url(remote)?;
    Some(match forge(&base) {
        Forge::GitLab => format!("{base}/-/commit/{commit}"),
        Forge::Bitbucket => format!("{base}/commits/{commit}"),
        Forge::GitHub => format!("{base}/commit/{commit}"),
    })
}

pub fn pull_request_url(remote: &str, number: u64) -> Option<String> {
    let base = web_base_url(remote)?;
    Some(match forge(&base) {
        Forge::GitLab => format!("{base}/-/merge_requests/{number}"),
        Forge::Bitbucket => format!("{base}/pull-requests/{number}"),
        Forge::GitHub => format!("{base}/pull/{number}"),
    })
}

pub fn pull_request_number(commit_message: &str) -> Option<u64> {
    let mut lines = commit_message.lines();
    let summary = lines.next()?;
    reference_number(summary).or_else(|| {
        lines
            .filter(|line| line.contains("merge request"))
            .find_map(reference_number)
    })
}

pub fn github_repo(remote: &str) -> Option<(String, String)> {
    let base = web_base_url(remote)?;
    let rest = base.strip_prefix("https://")?;
    let (host, repo) = rest.split_once('/')?;
    (host.contains("github") && repo.contains('/')).then(|| (host.to_owned(), repo.to_owned()))
}

fn reference_number(text: &str) -> Option<u64> {
    text.split(['#', '!'])
        .skip(1)
        .filter_map(leading_number)
        .last()
}

enum Forge {
    GitHub,
    GitLab,
    Bitbucket,
}

fn forge(base_url: &str) -> Forge {
    if base_url.contains("gitlab") {
        Forge::GitLab
    } else if base_url.contains("bitbucket") {
        Forge::Bitbucket
    } else {
        Forge::GitHub
    }
}

fn web_base_url(remote: &str) -> Option<String> {
    let remote = remote.trim_end_matches('/').trim_end_matches(".git");
    if let Some(rest) = remote
        .strip_prefix("https://")
        .or_else(|| remote.strip_prefix("http://"))
    {
        return Some(format!("https://{}", strip_user(rest)));
    }
    if let Some(rest) = remote.strip_prefix("ssh://") {
        let (host, path) = strip_user(rest).split_once('/')?;
        let host = host.split_once(':').map_or(host, |(host, _port)| host);
        return Some(format!("https://{host}/{path}"));
    }
    let (host, path) = strip_user(remote).split_once(':')?;
    Some(format!("https://{host}/{path}"))
}

fn strip_user(url: &str) -> &str {
    url.split_once('@').map_or(url, |(_, rest)| rest)
}

fn leading_number(text: &str) -> Option<u64> {
    let end = text.bytes().take_while(u8::is_ascii_digit).count();
    text[..end].parse().ok()
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn commit_url_from_scp_like_remote() {
        assert_eq!(
            commit_url("git@github.com:user/repo.git", "abc123").unwrap(),
            "https://github.com/user/repo/commit/abc123"
        );
    }

    #[test]
    fn commit_url_from_https_remote() {
        assert_eq!(
            commit_url("https://github.com/user/repo.git", "abc123").unwrap(),
            "https://github.com/user/repo/commit/abc123"
        );
    }

    #[test]
    fn commit_url_from_ssh_remote_with_port() {
        assert_eq!(
            commit_url("ssh://git@github.com:22/user/repo.git", "abc123").unwrap(),
            "https://github.com/user/repo/commit/abc123"
        );
    }

    #[test]
    fn commit_url_on_gitlab() {
        assert_eq!(
            commit_url("git@gitlab.com:user/repo.git", "abc123").unwrap(),
            "https://gitlab.com/user/repo/-/commit/abc123"
        );
    }

    #[test]
    fn commit_url_without_remote_host() {
        assert_eq!(commit_url("/local/repo", "abc123"), None);
    }

    #[test]
    fn pull_request_url_on_github() {
        assert_eq!(
            pull_request_url("git@github.com:user/repo.git", 42).unwrap(),
            "https://github.com/user/repo/pull/42"
        );
    }

    #[test]
    fn pull_request_url_on_bitbucket() {
        assert_eq!(
            pull_request_url("git@bitbucket.org:user/repo.git", 42).unwrap(),
            "https://bitbucket.org/user/repo/pull-requests/42"
        );
    }

    #[test]
    fn pull_request_number_from_squash_merge_message() {
        assert_eq!(
            pull_request_number("feat: add inline blame (#1234)"),
            Some(1234)
        );
    }

    #[test]
    fn pull_request_number_from_merge_commit_message() {
        assert_eq!(
            pull_request_number("Merge pull request #567 from user/branch"),
            Some(567)
        );
    }

    #[test]
    fn pull_request_number_without_reference() {
        assert_eq!(pull_request_number("feat: add inline blame"), None);
    }

    #[test]
    fn pull_request_number_from_gitlab_merge_message_body() {
        assert_eq!(
            pull_request_number(
                "Merge branch 'feature' into 'main'\n\nSee merge request group/project!42"
            ),
            Some(42)
        );
    }

    #[test]
    fn pull_request_number_ignores_issue_references_in_body() {
        assert_eq!(
            pull_request_number("Merge pull request #123 from user/branch\n\nFixes #45"),
            Some(123)
        );
    }

    #[test]
    fn github_repo_from_scp_like_remote() {
        assert_eq!(
            github_repo("git@github.com:user/repo.git"),
            Some(("github.com".to_owned(), "user/repo".to_owned()))
        );
    }

    #[test]
    fn github_repo_rejects_other_forges() {
        assert_eq!(github_repo("git@gitlab.com:user/repo.git"), None);
    }
}
