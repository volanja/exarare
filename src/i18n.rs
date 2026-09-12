//! Fixed text of the generated documents, per locale.
//!
//! Recorded data — commands, paths, diffs — is never translated. Only the
//! headings, labels and prompts around it come from here.

use anyhow::{Result, bail};

/// Locale of the generated document. English is the default, so that a runbook
/// reads the same regardless of the machine it was generated on.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Locale {
    #[default]
    En,
    Ja,
}

pub const ENV_LANG: &str = "EXARARE_LANG";

impl Locale {
    /// Accepts `en`, `ja` and the usual decorations: `en_US.UTF-8`, `ja-JP`.
    pub fn parse(value: &str) -> Result<Self> {
        let tag = value
            .split(['.', '@'])
            .next()
            .unwrap_or(value)
            .replace('_', "-")
            .to_ascii_lowercase();
        match tag.split('-').next().unwrap_or(&tag) {
            "en" => Ok(Locale::En),
            "ja" => Ok(Locale::Ja),
            _ => bail!("unsupported language {value}: exarare knows en and ja"),
        }
    }

    /// `--lang`, then `EXARARE_LANG`, then English. `LANG` is deliberately
    /// ignored: the output should not change with the environment.
    pub fn resolve(flag: Option<&str>) -> Result<Self> {
        if let Some(value) = flag {
            return Locale::parse(value);
        }
        match std::env::var(ENV_LANG) {
            Ok(value) if !value.trim().is_empty() => Locale::parse(&value),
            _ => Ok(Locale::default()),
        }
    }

    pub fn messages(self) -> Messages {
        Messages { locale: self }
    }
}

/// Every piece of fixed text in a generated document.
pub struct Messages {
    locale: Locale,
}

/// Picks the string for the current locale.
macro_rules! msg {
    ($self:ident, $en:expr, $ja:expr) => {
        match $self.locale {
            Locale::En => $en,
            Locale::Ja => $ja,
        }
    };
}

impl Messages {
    // --- Metadata ---

    pub fn runbook_title(&self) -> &'static str {
        msg!(self, "Build runbook", "構築手順書")
    }

    pub fn host(&self) -> &'static str {
        msg!(self, "Host", "対象ホスト")
    }

    pub fn user(&self) -> &'static str {
        msg!(self, "Ran as", "実行ユーザー")
    }

    pub fn shell(&self) -> &'static str {
        msg!(self, "Shell", "シェル")
    }

    pub fn started(&self) -> &'static str {
        msg!(self, "Started", "開始")
    }

    pub fn ended(&self) -> &'static str {
        msg!(self, "Ended", "終了")
    }

    pub fn session_id(&self) -> &'static str {
        msg!(self, "Session", "セッション")
    }

    // --- Chapter 1: purpose ---

    pub fn purpose(&self) -> &'static str {
        msg!(self, "Purpose", "目的")
    }

    pub fn purpose_intro(&self) -> &'static str {
        msg!(
            self,
            "Exarare cannot know why this work was done. Fill this in.",
            "この作業を行った理由は記録から分かりません。以下を記入してください。"
        )
    }

    pub fn purpose_what(&self) -> &'static str {
        msg!(self, "What was done", "何をしたか")
    }

    pub fn purpose_why(&self) -> &'static str {
        msg!(self, "Why it was needed", "なぜ必要だったか")
    }

    pub fn purpose_done(&self) -> &'static str {
        msg!(self, "Definition of done", "完了条件")
    }

    // --- Chapter 2: overview ---

    pub fn overview(&self) -> &'static str {
        msg!(self, "Overview of the work", "作業の全体像")
    }

    pub fn overview_intro(&self) -> &'static str {
        msg!(
            self,
            "The steps below are the order in which the work was done.",
            "実際に作業した順序です。"
        )
    }

    /// Title of the single step of a session where no heading was recorded.
    pub fn unnamed_step(&self) -> &'static str {
        msg!(self, "The whole session", "セッション全体")
    }

    /// Title of the work that came before the operator's first step.
    pub fn before_first_step(&self) -> &'static str {
        msg!(self, "Before the first step", "最初のステップより前の作業")
    }

    pub fn no_steps_hint(&self) -> &'static str {
        msg!(
            self,
            "No steps were recorded, so the session is a single step. Run `exarare step \"...\"` while working to split it up.",
            "ステップが記録されていないため、セッション全体が1ステップになっています。作業中に `exarare step \"...\"` を実行すると分割できます。"
        )
    }

    /// Label of a remark recorded with `exarare note`.
    pub fn note_label(&self) -> &'static str {
        msg!(self, "Note", "補足")
    }

    /// Separator between items written inline, which differs by script.
    pub fn list_separator(&self) -> &'static str {
        msg!(self, ", ", "、")
    }

    pub fn count_commands(&self, n: usize) -> String {
        msg!(
            self,
            if n == 1 {
                "1 command".to_string()
            } else {
                format!("{n} commands")
            },
            format!("コマンド {n} 件")
        )
    }

    pub fn count_packages(&self, n: usize) -> String {
        msg!(
            self,
            if n == 1 {
                "1 package".to_string()
            } else {
                format!("{n} packages")
            },
            format!("パッケージ {n} 件")
        )
    }

    pub fn count_files(&self, n: usize) -> String {
        msg!(
            self,
            if n == 1 {
                "1 changed file".to_string()
            } else {
                format!("{n} changed files")
            },
            format!("変更ファイル {n} 件")
        )
    }

    // --- Chapter 3: prerequisites ---

    pub fn prerequisites(&self) -> &'static str {
        msg!(self, "Prerequisites", "前提条件")
    }

    pub fn prereq_recorded(&self) -> &'static str {
        msg!(self, "Recorded from the session", "記録から分かること")
    }

    pub fn prereq_todo(&self) -> &'static str {
        msg!(
            self,
            "Add anything the record cannot show: network access, credentials, artifacts to have at hand.",
            "記録に残らない前提を補ってください。ネットワーク要件、必要な認証情報、事前に用意する資材など。"
        )
    }

    // --- Chapter 4: procedure ---

    pub fn procedure(&self) -> &'static str {
        msg!(self, "Procedure", "手順")
    }

    pub fn procedure_intro(&self) -> &'static str {
        msg!(
            self,
            "Only the commands that worked are listed here. Everything that ran, including failures and retries, is in the appendix.",
            "ここには成功したコマンドだけを載せています。失敗や再試行を含む全実行は付録にあります。"
        )
    }

    pub fn working_directory(&self) -> &'static str {
        msg!(self, "Working directory", "作業ディレクトリ")
    }

    pub fn step_files(&self) -> &'static str {
        msg!(self, "Files changed in this step", "このステップでの変更")
    }

    pub fn attribution_note(&self) -> &'static str {
        msg!(
            self,
            "A file is placed in a step when a command of that step names its path. The rest are listed under \"What changed\".",
            "ファイルは、そのパスを含むコマンドがあるステップに割り当てています。割り当てられなかったものは「変更されたもの」にまとめています。"
        )
    }

    // --- Chapter 5: what changed ---

    pub fn what_changed(&self) -> &'static str {
        msg!(self, "What changed", "変更されたもの")
    }

    pub fn changed_files(&self) -> &'static str {
        msg!(self, "Files", "ファイル")
    }

    pub fn packages(&self) -> &'static str {
        msg!(self, "Packages", "パッケージ")
    }

    // --- Touched files, from the watcher ---

    pub fn touched_files(&self) -> &'static str {
        msg!(self, "Files touched", "触られたファイル")
    }

    pub fn touched_intro(&self) -> &'static str {
        msg!(
            self,
            "These paths were written to during the work, and their content was not recorded: they are either outside the snapshotted directories, or they ended up unchanged.",
            "作業中に書き込みが発生したパスです。内容は記録していません。スナップショット対象の外にあるか、最終的に内容が変わらなかったものです。"
        )
    }

    pub fn touched_truncated(&self, limit: usize) -> String {
        msg!(
            self,
            format!("More than {limit} paths were touched, so this list is incomplete."),
            format!("{limit} 件を超えるパスが触られたため、この一覧は網羅的ではありません。")
        )
    }

    pub fn touched_incomplete(&self) -> &'static str {
        msg!(
            self,
            "Some directories could not be watched, so touches under them were missed:",
            "監視できなかったディレクトリがあり、その配下の書き込みは取りこぼしています。"
        )
    }

    pub fn step_touched(&self) -> &'static str {
        msg!(
            self,
            "Files touched in this step",
            "このステップで触られたファイル"
        )
    }

    /// Directories watched for touches only, with no content recorded.
    pub fn watched_directories(&self) -> &'static str {
        msg!(self, "Watched for touches", "監視のみの対象")
    }

    pub fn packages_installed(&self) -> &'static str {
        msg!(self, "Installed", "追加")
    }

    pub fn packages_removed(&self) -> &'static str {
        msg!(self, "Removed", "削除")
    }

    pub fn packages_upgraded(&self) -> &'static str {
        msg!(self, "Upgraded", "更新")
    }

    pub fn packages_dependencies(&self, n: usize) -> String {
        msg!(
            self,
            format!("{n} more were installed as dependencies"),
            format!("依存関係として {n} 件が追加されました")
        )
    }

    pub fn repositories_added(&self) -> &'static str {
        msg!(self, "Repositories enabled", "有効化されたリポジトリ")
    }

    pub fn repositories_removed(&self) -> &'static str {
        msg!(self, "Repositories disabled", "無効化されたリポジトリ")
    }

    pub fn modules_added(&self) -> &'static str {
        msg!(self, "Module streams enabled", "有効化されたモジュール")
    }

    pub fn no_package_changes(&self) -> &'static str {
        msg!(
            self,
            "No package changes were recorded.",
            "パッケージの変更は記録されていません。"
        )
    }

    pub fn packages_unavailable(&self) -> &'static str {
        msg!(
            self,
            "Package state was not recorded: this host has no rpm.",
            "パッケージの状態は記録されていません。このホストに rpm がありません。"
        )
    }

    /// Introduces the reasons the package picture may be incomplete.
    pub fn packages_incomplete(&self) -> &'static str {
        msg!(
            self,
            "Some package information could not be read:",
            "一部のパッケージ情報を取得できませんでした。"
        )
    }

    // --- Services, firewall, accounts ---

    pub fn services(&self) -> &'static str {
        msg!(self, "Services and firewall", "サービスとファイアウォール")
    }

    pub fn accounts(&self) -> &'static str {
        msg!(self, "Users and groups", "ユーザーとグループ")
    }

    pub fn units_enabled(&self) -> &'static str {
        msg!(self, "Enabled at boot", "自動起動を有効化")
    }

    pub fn units_disabled(&self) -> &'static str {
        msg!(self, "Disabled at boot", "自動起動を無効化")
    }

    pub fn services_started(&self) -> &'static str {
        msg!(self, "Running afterwards", "作業後に稼働中")
    }

    pub fn services_stopped(&self) -> &'static str {
        msg!(self, "No longer running", "稼働を停止")
    }

    pub fn firewall_services_added(&self) -> &'static str {
        msg!(self, "Firewall services opened", "許可したサービス")
    }

    pub fn firewall_services_removed(&self) -> &'static str {
        msg!(self, "Firewall services closed", "許可を解除したサービス")
    }

    pub fn firewall_ports_added(&self) -> &'static str {
        msg!(self, "Ports opened", "開放したポート")
    }

    pub fn firewall_ports_removed(&self) -> &'static str {
        msg!(self, "Ports closed", "閉じたポート")
    }

    pub fn users_added(&self) -> &'static str {
        msg!(self, "Users added", "追加したユーザー")
    }

    pub fn users_removed(&self) -> &'static str {
        msg!(self, "Users removed", "削除したユーザー")
    }

    pub fn users_changed(&self) -> &'static str {
        msg!(self, "Users changed", "変更したユーザー")
    }

    pub fn groups_added(&self) -> &'static str {
        msg!(self, "Groups added", "追加したグループ")
    }

    pub fn groups_removed(&self) -> &'static str {
        msg!(self, "Groups removed", "削除したグループ")
    }

    pub fn no_state_changes(&self) -> &'static str {
        msg!(
            self,
            "No service, firewall, user or group changes were recorded.",
            "サービス、ファイアウォール、ユーザー、グループの変更は記録されていません。"
        )
    }

    pub fn systemd_unavailable(&self) -> &'static str {
        msg!(
            self,
            "Service state was not recorded: systemd did not answer, as happens inside a container.",
            "サービスの状態は記録されていません。コンテナ内などで systemd が応答しなかったためです。"
        )
    }

    pub fn firewall_unavailable(&self) -> &'static str {
        msg!(
            self,
            "Firewall rules were not recorded: firewalld did not answer.",
            "ファイアウォールの設定は記録されていません。firewalld が応答しなかったためです。"
        )
    }

    pub fn step_state(&self) -> &'static str {
        msg!(
            self,
            "Services and accounts changed in this step",
            "このステップで変更したサービスとアカウント"
        )
    }

    pub fn rollback_state(&self) -> &'static str {
        msg!(
            self,
            "Services to disable and accounts to remove",
            "無効化するサービスと削除するアカウント"
        )
    }

    // --- Ansible playbook comments ---
    //
    // Task names stay English, because ansible-lint expects them to start with
    // a capital letter. These comments are what a reviewer reads.

    pub fn playbook_header(&self, session: &str, version: &str) -> String {
        msg!(
            self,
            format!("Generated by exarare {version} from session {session}."),
            format!("exarare {version} がセッション {session} から生成しました。")
        )
    }

    pub fn playbook_review(&self) -> &'static str {
        msg!(
            self,
            "Review before running. Tasks come from what was observed before and after the work, not from a declaration of intent. A command that no probe could express is replayed as it was typed and is not idempotent.\nThe firewall tasks need the ansible.posix collection.",
            "実行する前に内容を確認してください。タスクは作業の前後で観測した差分から作られており、意図の宣言ではありません。観測で表せなかったコマンドは入力どおり再実行されるため、冪等ではありません。\nファイアウォールのタスクには ansible.posix コレクションが必要です。"
        )
    }

    pub fn playbook_step(&self, title: &str) -> String {
        msg!(self, format!("Step: {title}"), format!("ステップ: {title}"))
    }

    pub fn playbook_unattributed(&self) -> &'static str {
        msg!(
            self,
            "Changes that belong to no single step.",
            "特定のステップに割り当てられなかった変更。"
        )
    }

    pub fn playbook_empty(&self) -> &'static str {
        msg!(
            self,
            "The session recorded nothing that can be replayed.",
            "再実行できる変更は記録されていません。"
        )
    }

    pub fn todo_idempotent(&self) -> &'static str {
        msg!(
            self,
            "TODO: make idempotent. This command is replayed as it was typed.",
            "要対応: 冪等ではありません。このコマンドは入力どおり再実行されます。"
        )
    }

    pub fn todo_content_missing(&self, path: &str) -> String {
        msg!(
            self,
            format!(
                "TODO: {path} changed, but its content was not recorded (secret, binary or too large). Set it by hand."
            ),
            format!(
                "要対応: {path} は変更されましたが、内容を記録していません（機密、バイナリ、またはサイズ超過）。手動で設定してください。"
            )
        )
    }

    pub fn recorded_owner(&self, uid: u32, gid: u32) -> String {
        msg!(
            self,
            format!(
                "Recorded owner: uid {uid}, gid {gid}. Set owner and group by name if that matters on the target."
            ),
            format!(
                "記録時の所有者: uid {uid}、gid {gid}。対象ホストで意味がある場合は名前で owner と group を指定してください。"
            )
        )
    }

    pub fn name(&self) -> &'static str {
        msg!(self, "Name", "名前")
    }

    pub fn version(&self) -> &'static str {
        msg!(self, "Version", "バージョン")
    }

    pub fn no_changes(&self) -> &'static str {
        msg!(
            self,
            "No file changes were recorded.",
            "ファイルの変更は記録されていません。"
        )
    }

    pub fn change_kind(&self, label: &str) -> &'static str {
        match (self.locale, label) {
            (Locale::En, "added") => "added",
            (Locale::En, "removed") => "removed",
            (Locale::En, "modified") => "modified",
            (Locale::En, _) => "permissions",
            (Locale::Ja, "added") => "追加",
            (Locale::Ja, "removed") => "削除",
            (Locale::Ja, "modified") => "変更",
            (Locale::Ja, _) => "権限",
        }
    }

    pub fn path(&self) -> &'static str {
        msg!(self, "Path", "パス")
    }

    pub fn change(&self) -> &'static str {
        msg!(self, "Change", "変更")
    }

    pub fn diff_truncated(&self, shown: usize, total: usize) -> String {
        msg!(
            self,
            format!("Diff truncated: {shown} of {total} lines. The full diff is in the appendix."),
            format!("差分を {total} 行中 {shown} 行までに省略しました。全文は付録にあります。")
        )
    }

    // --- Chapter 6: verification ---

    pub fn verification(&self) -> &'static str {
        msg!(self, "Verification", "確認手順")
    }

    pub fn verification_intro(&self) -> &'static str {
        msg!(
            self,
            "These commands were run during the work and look like checks. Write down what each should show.",
            "作業中に実行された、確認と思われるコマンドです。それぞれの期待結果を記入してください。"
        )
    }

    pub fn verification_none(&self) -> &'static str {
        msg!(
            self,
            "No commands looked like checks. Add the ones a reader should run.",
            "確認に見えるコマンドはありませんでした。読み手が実行すべき確認を追加してください。"
        )
    }

    pub fn expected_result(&self) -> &'static str {
        msg!(self, "Expected result", "期待結果")
    }

    // --- Chapter 7: rollback ---

    pub fn rollback(&self) -> &'static str {
        msg!(self, "Rollback", "切り戻し")
    }

    pub fn rollback_disclaimer(&self) -> &'static str {
        msg!(
            self,
            "**These steps are generated candidates, not a verified rollback procedure.** Removing a package does not undo what installing it changed. Restoring a file does not undo what a service did while it was running. The order in which things must be undone cannot be derived from the record. Review and test every step before relying on it.",
            "**以下は自動生成した候補であり、検証された切り戻し手順ではありません。** パッケージを削除しても、インストール時に加わった変更は元に戻りません。ファイルを戻しても、その設定で動作していたサービスの処理は取り消せません。どの順序で戻すべきかは記録からは導けません。実際に使う前に、必ず内容を確認し、試してください。"
        )
    }

    pub fn rollback_packages(&self) -> &'static str {
        msg!(self, "Packages to remove", "削除するパッケージ")
    }

    pub fn rollback_files(&self) -> &'static str {
        msg!(
            self,
            "Files to restore to their recorded content",
            "記録時点の内容に戻すファイル"
        )
    }

    pub fn rollback_none(&self) -> &'static str {
        msg!(
            self,
            "Nothing was detected that would need undoing.",
            "戻す必要がある変更は検出されませんでした。"
        )
    }

    // --- Appendices ---

    pub fn appendix_log(&self) -> &'static str {
        msg!(
            self,
            "Appendix A: full command log",
            "付録A: 全コマンド記録"
        )
    }

    pub fn appendix_log_intro(&self) -> &'static str {
        msg!(
            self,
            "Everything that ran, in order, including failures and retries.",
            "実行されたすべてのコマンドを、失敗や再試行も含めて順に並べています。"
        )
    }

    pub fn appendix_diffs(&self) -> &'static str {
        msg!(
            self,
            "Appendix B: full diffs of the files shortened above",
            "付録B: 本文で省略した差分の全文"
        )
    }

    pub fn appendix_generated(&self) -> &'static str {
        msg!(
            self,
            "Appendix B: how this was generated",
            "付録B: この文書の生成条件"
        )
    }

    pub fn time(&self) -> &'static str {
        msg!(self, "Time", "時刻")
    }

    pub fn command(&self) -> &'static str {
        msg!(self, "Command", "コマンド")
    }

    pub fn exit_code(&self) -> &'static str {
        msg!(self, "Exit", "終了コード")
    }

    pub fn still_running(&self) -> &'static str {
        msg!(self, "(no exit recorded)", "（終了コードなし）")
    }

    /// Directories whose content is snapshotted and diffed.
    pub fn snapshot_directories(&self) -> &'static str {
        msg!(
            self,
            "Snapshotted directories",
            "スナップショット対象ディレクトリ"
        )
    }

    pub fn generated_by(&self) -> &'static str {
        msg!(self, "Generated by", "生成ツール")
    }

    pub fn generated_note(&self) -> &'static str {
        msg!(
            self,
            "Files that change on their own are excluded, and the content of files matching a secret pattern is never recorded.",
            "自動的に変化するファイルは除外し、機密と判定されたファイルの内容は記録していません。"
        )
    }

    pub fn todo(&self) -> &'static str {
        msg!(self, "TODO", "要記入")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_language_tags() {
        for value in ["en", "EN", "en-US", "en_US.UTF-8"] {
            assert_eq!(Locale::parse(value).unwrap(), Locale::En, "{value}");
        }
        for value in ["ja", "ja-JP", "ja_JP.UTF-8", "ja_JP.eucjp@custom"] {
            assert_eq!(Locale::parse(value).unwrap(), Locale::Ja, "{value}");
        }
        assert!(Locale::parse("de").is_err());
        assert!(Locale::parse("C").is_err());
    }

    #[test]
    fn defaults_to_english() {
        assert_eq!(Locale::default(), Locale::En);
        assert_eq!(Locale::resolve(Some("ja")).unwrap(), Locale::Ja);
    }

    #[test]
    fn every_message_differs_by_locale() {
        // A missed translation shows up as the same string in both locales.
        let en = Locale::En.messages();
        let ja = Locale::Ja.messages();
        let pairs: Vec<(&str, &str)> = vec![
            (en.runbook_title(), ja.runbook_title()),
            (en.purpose(), ja.purpose()),
            (en.overview(), ja.overview()),
            (en.prerequisites(), ja.prerequisites()),
            (en.procedure(), ja.procedure()),
            (en.what_changed(), ja.what_changed()),
            (en.verification(), ja.verification()),
            (en.rollback(), ja.rollback()),
            (en.rollback_disclaimer(), ja.rollback_disclaimer()),
            (en.appendix_log(), ja.appendix_log()),
            (en.appendix_generated(), ja.appendix_generated()),
            (en.todo(), ja.todo()),
        ];
        for (english, japanese) in pairs {
            assert_ne!(english, japanese, "untranslated: {english}");
        }
    }
}
