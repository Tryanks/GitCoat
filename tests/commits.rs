//! Commit history and commit/diff pages.

mod common;

use std::collections::HashSet;

use common::{fixtures::TempRepo, get, test_app};
use gitcoat::app::url::{commit_url, commits_page_url};
use http::StatusCode;

/// Every `/commit/<oid>` link target in a history page, in document order.
fn row_oids(html: &str) -> Vec<String> {
    let mut oids = Vec::new();
    for row in html.split("<li class=\"commit-row\">").skip(1) {
        let link = row
            .find("href=\"/commit/")
            .expect("row links to its commit");
        let rest = &row[link + "href=\"/commit/".len()..];
        let end = rest.find('"').unwrap();
        oids.push(rest[..end].to_owned());
    }
    oids
}

fn count(html: &str, needle: &str) -> usize {
    html.matches(needle).count()
}

fn pager(html: &str) -> &str {
    let start = html.find("<nav class=\"pager\"").expect("pager");
    let end = html[start..].find("</nav>").unwrap();
    &html[start..start + end]
}

fn commits_page(reference: &str, at: &str, page: usize) -> String {
    commits_page_url(reference, at, page)
}

fn diff_file(html: &str, index: usize) -> &str {
    let marker = format!("<section class=\"diff-file\" id=\"diff-{index}\">");
    let start = html
        .find(&marker)
        .unwrap_or_else(|| panic!("diff file {index}"));
    let end = html[start..].find("</section>").unwrap();
    &html[start..start + end]
}

// --- History ---------------------------------------------------------------

#[tokio::test]
async fn first_page_lists_fifty_commits_and_pins_at() {
    let repo = TempRepo::rich();
    let head = repo.head_oid();
    let app = test_app(&repo.path);
    let response = get(&app, "/commits").await;
    assert_eq!(response.status, StatusCode::OK);
    let html = &response.body;

    assert!(
        html.contains("class=\"tab\" href=\"/commits\" aria-current=\"page\""),
        "Commits tab is active"
    );
    assert!(html.contains("Commits on main"), "heading names the ref");
    assert!(
        html.contains("class=\"ref-picker__item\" data-kind=\"branch\" href=\"/commits?ref=refs%2Fheads%2Fmain\" aria-selected=\"true\""),
        "ref picker links to history and marks main"
    );

    let oids = row_oids(html);
    assert_eq!(oids.len(), 50, "one full page");
    assert_eq!(oids[0], head, "newest first");
    assert_eq!(count(html, "<li class=\"commit-row\">"), 50);
    assert!(
        html.contains("Make scripts/run.sh executable</a>"),
        "titles are linked"
    );
    assert!(
        html.contains("<strong>Alice Example</strong>"),
        "author name"
    );
    assert!(
        html.contains(&format!(
            "<a class=\"oid mono\" href=\"{}\">{}</a>",
            commit_url(&head),
            &head[..7]
        )),
        "short oid link"
    );
    assert!(html.contains("<time datetime=\""), "relative time element");
    assert!(
        html.contains("class=\"badge\">merge<"),
        "merge badge on the merge commit"
    );

    let nav = pager(html);
    assert!(
        nav.contains("<span class=\"btn\" aria-disabled=\"true\">Newer</span>"),
        "Newer is disabled on page 1: {nav}"
    );
    let older = commits_page("refs/heads/main", &head, 2);
    assert!(
        nav.contains(&format!("href=\"{}\"", older.replace('&', "&amp;"))),
        "Older carries at=<head>&page=2: {nav}"
    );
    assert!(nav.contains("Page 1"));
}

#[tokio::test]
async fn pages_cover_the_whole_history_without_duplicates() {
    let repo = TempRepo::rich();
    let head = repo.head_oid();
    let app = test_app(&repo.path);

    let mut all = Vec::new();
    for page in 1..=3 {
        let response = get(&app, &commits_page("refs/heads/main", &head, page)).await;
        assert_eq!(response.status, StatusCode::OK, "page {page}");
        let oids = row_oids(&response.body);
        let nav = pager(&response.body);
        if page < 3 {
            assert_eq!(oids.len(), 50, "page {page} is full");
            assert!(nav.contains(">Older</a>"), "page {page} links older");
        } else {
            assert_eq!(oids.len(), 30, "last page holds the remainder");
            assert!(
                nav.contains("<span class=\"btn\" aria-disabled=\"true\">Older</span>"),
                "Older is disabled on the last page: {nav}"
            );
            let newer = commits_page("refs/heads/main", &head, 2);
            assert!(
                nav.contains(&format!("href=\"{}\"", newer.replace('&', "&amp;"))),
                "Newer links to page 2"
            );
        }
        assert!(nav.contains(&format!("Page {page}")));
        all.extend(oids);
    }
    assert_eq!(all.len(), 130);
    let unique: HashSet<&String> = all.iter().collect();
    assert_eq!(unique.len(), 130, "no commit appears twice");
    assert!(
        all.contains(&repo.root_oid),
        "root commit is on the last page"
    );
    assert!(
        all.contains(&repo.feature_oid),
        "merged branch commit is included"
    );
}

#[tokio::test]
async fn pinned_pages_are_stable_across_pushes() {
    let mut repo = TempRepo::rich();
    let head = repo.head_oid();
    let app = test_app(&repo.path);

    let before = row_oids(
        &get(&app, &commits_page("refs/heads/main", &head, 2))
            .await
            .body,
    );
    assert_eq!(before.len(), 50);

    let pushed = repo.push_new_commit_from_clone("Pushed after the app started");

    let after = row_oids(
        &get(&app, &commits_page("refs/heads/main", &head, 2))
            .await
            .body,
    );
    assert_eq!(before, after, "a pinned page does not shift");

    let fresh = get(&app, "/commits?ref=refs%2Fheads%2Fmain").await;
    assert_eq!(fresh.status, StatusCode::OK);
    let oids = row_oids(&fresh.body);
    assert_eq!(
        oids[0], pushed,
        "an unpinned first page sees the new commit"
    );
    assert!(
        fresh.body.contains(&format!("at={pushed}&amp;page=2")),
        "pager now pins the new head"
    );
}

#[tokio::test]
async fn bad_page_numbers() {
    let repo = TempRepo::rich();
    let head = repo.head_oid();
    let app = test_app(&repo.path);

    let zero = get(&app, "/commits?page=0").await;
    assert_eq!(zero.status, StatusCode::BAD_REQUEST);
    assert!(zero.body.contains("class=\"error-page\""), "branded 400");

    let text = get(&app, "/commits?page=two").await;
    assert_eq!(text.status, StatusCode::BAD_REQUEST);

    let far = get(&app, &commits_page("refs/heads/main", &head, 999)).await;
    assert_eq!(far.status, StatusCode::NOT_FOUND);
    assert!(far.body.contains("No commits on this page"), "empty state");
    assert!(
        far.body
            .contains(&commits_page("refs/heads/main", &head, 1).replace('&', "&amp;")),
        "links back to page 1"
    );
    assert!(
        far.body.contains("class=\"ref-picker\""),
        "toolbar still rendered"
    );
}

#[tokio::test]
async fn bad_at_and_ref_are_404() {
    let repo = TempRepo::rich();
    let app = test_app(&repo.path);

    let bad = get(&app, "/commits?at=deadbeef").await;
    assert_eq!(bad.status, StatusCode::NOT_FOUND);
    assert!(bad.body.contains("class=\"error-page\""));

    // A well-formed id that is not an object.
    let missing = get(&app, &format!("/commits?at={}", "0".repeat(40))).await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);

    // A blob id is not a commit.
    let blob = repo.rev_parse("HEAD:README.md");
    let not_commit = get(&app, &format!("/commits?at={blob}")).await;
    assert_eq!(not_commit.status, StatusCode::NOT_FOUND);

    let no_ref = get(&app, "/commits?ref=refs%2Fheads%2Fnope").await;
    assert_eq!(no_ref.status, StatusCode::NOT_FOUND);
    assert!(no_ref.body.contains("Ref not found"));

    let revspec = get(&app, "/commits?ref=main").await;
    assert_eq!(
        revspec.status,
        StatusCode::NOT_FOUND,
        "short names are not refs"
    );
}

#[tokio::test]
async fn branch_with_slash_tags_and_duplicate_names() {
    let repo = TempRepo::rich();
    let app = test_app(&repo.path);

    let feature = get(&app, "/commits?ref=refs%2Fheads%2Ffeature%2Fwith-slash").await;
    assert_eq!(feature.status, StatusCode::OK);
    let html = &feature.body;
    assert_eq!(row_oids(html)[0], repo.feature_oid);
    assert!(html.contains("Commits on feature/with-slash"));
    assert!(
        html.contains(
            "href=\"/commits?ref=refs%2Fheads%2Ffeature%2Fwith-slash\" aria-selected=\"true\""
        ),
        "picker marks the branch"
    );
    assert!(
        html.contains("class=\"tab\" href=\"/commits?ref=refs%2Fheads%2Ffeature%2Fwith-slash\" aria-current=\"page\""),
        "tab keeps the ref"
    );
    assert_eq!(
        row_oids(html).len(),
        6,
        "five shared commits plus the branch's own"
    );
    assert!(
        pager(html).contains("<span class=\"btn\" aria-disabled=\"true\">Older</span>"),
        "short history fits one page"
    );

    let dup_branch = get(&app, "/commits?ref=refs%2Fheads%2Fdup").await;
    let dup_tag = get(&app, "/commits?ref=refs%2Ftags%2Fdup").await;
    assert_eq!(row_oids(&dup_branch.body)[0], repo.dup_branch_oid);
    assert_eq!(row_oids(&dup_tag.body)[0], repo.dup_tag_oid);
    assert_ne!(repo.dup_branch_oid, repo.dup_tag_oid);
    assert!(
        dup_tag.body.contains(
            "data-kind=\"tag\" href=\"/commits?ref=refs%2Ftags%2Fdup\" aria-selected=\"true\""
        ),
        "tag selected in the picker"
    );
    assert!(
        dup_tag
            .body
            .contains("data-kind=\"tag\" aria-pressed=\"true\""),
        "Tags tab opens first for a tag"
    );

    let annotated = get(&app, "/commits?ref=refs%2Ftags%2Fv1.0.0").await;
    assert_eq!(annotated.status, StatusCode::OK);
    assert_eq!(
        row_oids(&annotated.body)[0],
        repo.v1_oid,
        "annotated tag peels"
    );
    assert!(annotated.body.contains("Commits on v1.0.0"));

    let by_oid = get(&app, &format!("/commits?ref={}", repo.root_oid)).await;
    assert_eq!(by_oid.status, StatusCode::OK);
    let oids = row_oids(&by_oid.body);
    assert_eq!(oids, vec![repo.root_oid.clone()]);
    assert!(
        by_oid
            .body
            .contains("<span class=\"btn\" aria-disabled=\"true\">Older</span>")
    );
}

#[tokio::test]
async fn bare_repository_history() {
    let repo = TempRepo::rich_bare();
    let app = test_app(&repo.path);
    let response = get(&app, "/commits").await;
    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(row_oids(&response.body).len(), 50);
}

#[tokio::test]
async fn empty_repository_shows_empty_state() {
    let repo = TempRepo::empty_bare();
    let app = test_app(&repo.path);
    let response = get(&app, "/commits").await;
    assert_eq!(response.status, StatusCode::OK);
    assert!(response.body.contains("class=\"empty-state\""));
    assert!(response.body.contains("No commits yet"));
    assert!(!response.body.contains("class=\"commits\""));
}

// --- Commit page -----------------------------------------------------------

#[tokio::test]
async fn root_commit_is_diffed_against_the_empty_tree() {
    let repo = TempRepo::rich();
    let app = test_app(&repo.path);
    let response = get(&app, &commit_url(&repo.root_oid)).await;
    assert_eq!(response.status, StatusCode::OK);
    let html = &response.body;

    assert!(html.contains("<h1 class=\"commit-header__title\">Initial commit</h1>"));
    assert!(
        html.contains(&format!(
            "<span class=\"oid mono\">{}</span>",
            repo.root_oid
        )),
        "full oid"
    );
    assert!(
        html.contains(&format!("data-copy=\"{}\"", repo.root_oid)),
        "copy button"
    );
    assert!(
        html.contains(&format!(
            "href=\"/tree?ref={}\">Browse files</a>",
            repo.root_oid
        )),
        "browse link"
    );
    assert!(
        html.contains("Alice Example &lt;alice@example.com&gt;"),
        "author with escaped email"
    );
    assert!(
        !html.contains("<dt>Committer</dt>"),
        "same committer is not repeated"
    );
    assert!(html.contains("Root commit (compared against the empty tree)"));
    assert!(!html.contains("Merge commit"));
    assert!(
        html.contains("class=\"tab\" href=\"/commits\" aria-current=\"page\""),
        "Commits tab active"
    );

    assert!(html.contains("4 files changed"), "{html}");
    assert!(html.contains("href=\"#diff-0\">README.md</a><span class=\"badge\">Added</span>"));
    assert!(html.contains("href=\"#diff-3\">src/main.rs</a>"));
    let readme = diff_file(html, 0);
    assert!(readme.contains("<span class=\"badge\">Added</span>"));
    assert!(readme.contains("<span class=\"stat-add\">+31</span>"));
    assert!(
        readme.contains("<tr class=\"diff__hunk\"><td colspan=\"4\">@@ -0,0 +1,31 @@</td></tr>")
    );
    assert!(readme.contains(
        "<tr class=\"diff__add\"><td class=\"diff__ln\"></td><td class=\"diff__ln\">1</td><td class=\"diff__sign\">+</td><td class=\"diff__code\"># GitCoat fixture</td></tr>"
    ));
    assert!(
        readme.contains(&format!(
            "href=\"/blob?ref={}&amp;path=README.md\"",
            repo.root_oid
        )),
        "link to the file at this commit"
    );
    let empty = diff_file(html, 2);
    assert!(empty.contains("empty.txt"));
    assert!(
        empty.contains("No content changes"),
        "empty file has no hunks"
    );
}

#[tokio::test]
async fn merge_commit_shows_first_parent_changes() {
    let repo = TempRepo::rich();
    let app = test_app(&repo.path);
    let response = get(&app, &commit_url(&repo.merge_oid)).await;
    assert_eq!(response.status, StatusCode::OK);
    let html = &response.body;

    let parents: Vec<String> = repo
        .git(&["rev-list", "--parents", "-n", "1", &repo.merge_oid])
        .split_whitespace()
        .skip(1)
        .map(str::to_owned)
        .collect();
    assert_eq!(parents.len(), 2);
    for parent in &parents {
        assert!(
            html.contains(&format!(
                "<a class=\"oid mono\" href=\"{}\">{}</a>",
                commit_url(parent),
                &parent[..7]
            )),
            "parent link {parent}"
        );
    }
    assert!(html.contains(&format!(
        "Merge commit: showing changes against the first parent <span class=\"oid mono\">{}</span>",
        &parents[0][..7]
    )));
    assert!(
        !html.contains("No changes"),
        "the merge brought feature.txt in"
    );
    assert!(html.contains("1 file changed"));
    assert!(html.contains("href=\"#diff-0\">feature.txt</a><span class=\"badge\">Added</span>"));
    assert!(html.contains("<td class=\"diff__code\">feature branch content</td>"));
}

#[tokio::test]
async fn rename_delete_mode_change_and_binary() {
    let repo = TempRepo::rich();
    let app = test_app(&repo.path);

    let rename = get(&app, &commit_url(&repo.rename_oid)).await.body;
    assert!(rename.contains("<span class=\"badge\">Renamed</span>"));
    assert!(rename.contains("docs/guide.md → docs/GUIDE.md"));
    let file = diff_file(&rename, 0);
    assert!(
        !file.contains("<table class=\"diff\">"),
        "no hunks for a pure rename"
    );
    assert!(!file.contains("stat-add"), "no fabricated counts: {file}");
    assert!(file.contains("File renamed without content changes"));
    assert!(
        file.contains("path=docs%2FGUIDE.md\""),
        "links the new path"
    );

    let delete = get(&app, &commit_url(&repo.delete_oid)).await.body;
    assert!(delete.contains("<span class=\"badge\">Deleted</span>"));
    assert!(delete.contains("href=\"#diff-0\">notes/todo.txt</a>"));
    let file = diff_file(&delete, 0);
    assert!(file.contains(
        "<tr class=\"diff__del\"><td class=\"diff__ln\">1</td><td class=\"diff__ln\"></td><td class=\"diff__sign\">-</td><td class=\"diff__code\">- write tests</td></tr>"
    ));
    assert!(file.contains("<span class=\"stat-del\">−1</span>"));
    assert!(
        !file.contains("View file"),
        "a deleted file has no blob view"
    );

    let mode = get(&app, &commit_url(&repo.mode_change_oid)).await.body;
    assert!(mode.contains("<span class=\"badge\">Mode changed</span>"));
    assert!(mode.contains("100644 → 100755"));
    let file = diff_file(&mode, 0);
    assert!(file.contains("scripts/run.sh"));
    assert!(
        !file.contains("<table class=\"diff\">"),
        "no hunks for a mode change"
    );
    assert!(file.contains("Only the file mode changed"));

    let binary = get(&app, &commit_url(&repo.binary_oid)).await.body;
    assert!(binary.contains("2 files changed"));
    assert!(
        !binary.contains("stat-add"),
        "no counts for binaries: {binary}"
    );
    for (index, path) in [(0, "assets/logo.png"), (1, "bin/blob.bin")] {
        let file = diff_file(&binary, index);
        assert!(file.contains(path), "{path}");
        assert!(file.contains("<span class=\"muted\">binary</span>"));
        assert!(file.contains("Binary file not shown"));
        assert!(!file.contains("<table class=\"diff\">"));
    }
}

#[tokio::test]
async fn modification_has_old_and_new_line_numbers() {
    let repo = TempRepo::rich();
    let app = test_app(&repo.path);
    // The counter commits rewrite the single line of counter.txt.
    let oid = repo.rev_parse("refs/heads/main~5");
    let response = get(&app, &commit_url(&oid)).await;
    assert_eq!(response.status, StatusCode::OK);
    let html = &response.body;
    assert!(
        html.contains("<h1 class=\"commit-header__title\">Bump counter to 119</h1>"),
        "{html}"
    );
    let file = diff_file(html, 0);
    assert!(file.contains("<span class=\"badge\">Modified</span>"));
    assert!(file.contains("<tr class=\"diff__hunk\"><td colspan=\"4\">@@ -1,1 +1,1 @@</td></tr>"));
    assert!(file.contains(
        "<tr class=\"diff__del\"><td class=\"diff__ln\">1</td><td class=\"diff__ln\"></td><td class=\"diff__sign\">-</td><td class=\"diff__code\">118</td></tr>"
    ));
    assert!(file.contains(
        "<tr class=\"diff__add\"><td class=\"diff__ln\"></td><td class=\"diff__ln\">1</td><td class=\"diff__sign\">+</td><td class=\"diff__code\">119</td></tr>"
    ));
    assert!(file.contains("<span class=\"stat-add\">+1</span><span class=\"stat-del\">−1</span>"));

    // A larger change keeps context lines numbered on both sides.
    let response = get(&app, &commit_url(&repo.rev_parse("refs/heads/main~126"))).await;
    let html = &response.body;
    assert!(html.contains("Add odd file names, HTML and SVG"), "{html}");
    let file = diff_file(html, 0);
    assert!(file.contains("<tr class=\"diff__add\">"));
}

#[tokio::test]
async fn context_lines_carry_both_numbers() {
    let mut repo = TempRepo::rich();
    let oid = repo.commit_files(
        &[(
            "README.md",
            b"# GitCoat fixture\n\nChanged line\n\n## Features\n",
        )],
        "Edit the README",
    );
    let app = test_app(&repo.path);
    let html = get(&app, &commit_url(&oid)).await.body;
    let file = diff_file(&html, 0);
    assert!(file.contains(
        "<tr class=\"diff__ctx\"><td class=\"diff__ln\">1</td><td class=\"diff__ln\">1</td><td class=\"diff__sign\"> </td><td class=\"diff__code\"># GitCoat fixture</td></tr>"
    ), "{file}");
    assert!(file.contains(
        "<tr class=\"diff__add\"><td class=\"diff__ln\"></td><td class=\"diff__ln\">3</td><td class=\"diff__sign\">+</td><td class=\"diff__code\">Changed line</td></tr>"
    ));
}

#[tokio::test]
async fn unknown_and_abbreviated_oids_are_404() {
    let repo = TempRepo::rich();
    let app = test_app(&repo.path);

    let missing = get(&app, &format!("/commit/{}", "0".repeat(40))).await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
    assert!(missing.body.contains("class=\"error-page\""), "branded 404");

    let short = get(&app, &format!("/commit/{}", &repo.root_oid[..7])).await;
    assert_eq!(
        short.status,
        StatusCode::NOT_FOUND,
        "abbreviations are not resolved"
    );
    assert!(short.body.contains("class=\"error-page\""));

    let junk = get(&app, "/commit/not-a-hash").await;
    assert_eq!(junk.status, StatusCode::NOT_FOUND);

    let blob = repo.rev_parse("HEAD:README.md");
    let not_commit = get(&app, &commit_url(&blob)).await;
    assert_eq!(not_commit.status, StatusCode::NOT_FOUND);

    // The annotated tag object is not a commit; the tag's history is reached
    // through /commits?ref=refs/tags/... instead.
    let tag = get(&app, &commit_url(&repo.v1_tag_oid)).await;
    assert_eq!(tag.status, StatusCode::NOT_FOUND);

    let empty = get(&app, "/commit/").await;
    assert_eq!(empty.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn messages_and_identities_are_escaped() {
    let repo = TempRepo::rich();
    // fast-import lets the stream choose the identity (the fixture's env
    // would otherwise pin it to Alice).
    let head = repo.head_oid();
    let message = "<script>alert(1)</script> title\n\nBody with <b>tags</b> & ampersands\n";
    let stream = format!(
        "commit refs/heads/main\n\
         author Eve & \"Mallory\" <eve&mal@example.com> 1700000000 +0000\n\
         committer Eve & \"Mallory\" <eve&mal@example.com> 1700000000 +0000\n\
         data {}\n{message}\nfrom {head}\nM 100644 inline x.txt\ndata 2\nx\n\ndone\n",
        message.len()
    );
    repo.git_stdin(&["fast-import", "--quiet", "--done"], stream.as_bytes());
    let oid = repo.head_oid();
    assert_ne!(oid, head);
    let app = test_app(&repo.path);

    let page = get(&app, &commit_url(&oid)).await.body;
    assert!(
        page.contains(
            "<h1 class=\"commit-header__title\">&lt;script&gt;alert(1)&lt;/script&gt; title</h1>"
        ),
        "{page}"
    );
    assert!(page.contains(
        "<pre class=\"commit-header__body\">Body with &lt;b&gt;tags&lt;/b&gt; &amp; ampersands</pre>"
    ));
    assert!(!page.contains("<script>alert(1)</script>"));
    assert!(
        page.contains("Eve &amp; &quot;Mallory&quot; &lt;eve&amp;mal@example.com&gt;")
            || page.contains("Eve &amp; \"Mallory\" &lt;eve&amp;mal@example.com&gt;"),
        "author escaped: {page}"
    );

    let list = get(&app, "/commits").await.body;
    assert!(list.contains("&lt;script&gt;alert(1)&lt;/script&gt; title</a>"));
    assert!(
        list.contains("<strong>Eve &amp; &quot;Mallory&quot;</strong>")
            || list.contains("<strong>Eve &amp; \"Mallory\"</strong>")
    );
    assert!(!list.contains("<script>alert(1)</script>"));
}
