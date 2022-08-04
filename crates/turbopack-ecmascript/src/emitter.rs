use swc_common::{
    errors::{DiagnosticBuilder, DiagnosticId, Emitter, Level},
    source_map::Pos,
};
use turbo_tasks::{emit, primitives::StringVc};
use turbopack_core::{
    asset::AssetVc,
    issue::{
        analyze::{AnalyzeIssue, AnalyzeIssueVc},
        IssueSeverity, IssueSourceVc, IssueVc,
    },
};

pub struct IssueEmitter {
    pub source: AssetVc,
    pub title: Option<String>,
}

impl Emitter for IssueEmitter {
    fn emit(&mut self, db: &DiagnosticBuilder<'_>) {
        let level = db.level;
        let mut message = db
            .message
            .iter()
            .map(|(s, _)| s.as_ref())
            .collect::<Vec<_>>()
            .join("");
        let code = db.code.as_ref().map(|d| match d {
            DiagnosticId::Error(s) => format!("error {s}"),
            DiagnosticId::Lint(s) => format!("lint {s}"),
        });

        let title;
        if let Some(t) = self.title.as_ref() {
            title = t.clone();
        } else {
            let mut message_split = message.split('\n');
            title = message_split.next().unwrap().to_string();
            message = message_split.as_str().to_string();
        }

        let source = db.span.primary_span().map(|span| {
            IssueSourceVc::from_byte_offset(
                self.source,
                span.lo().to_usize().saturating_sub(1),
                span.hi().to_usize().saturating_sub(1),
            )
        });
        // TODO add other primary and secondary spans with labels as sub_issues

        let issue: AnalyzeIssueVc = AnalyzeIssue {
            severity: match level {
                Level::Bug => IssueSeverity::Bug,
                Level::Fatal | Level::PhaseFatal => IssueSeverity::Fatal,
                Level::Error => IssueSeverity::Error,
                Level::Warning => IssueSeverity::Warning,
                Level::Note => IssueSeverity::Note,
                Level::Help => IssueSeverity::Hint,
                Level::Cancelled => IssueSeverity::Error,
                Level::FailureNote => IssueSeverity::Note,
            }
            .into(),
            path: self.source.path(),
            title: StringVc::cell(title),
            message: StringVc::cell(message),
            code,
            source,
        }
        .into();
        emit::<IssueVc>(issue.into());
    }
}
