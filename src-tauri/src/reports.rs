use crate::{acp_client::DiagnoseEvent, extractor::MetricsSnapshot};
use serde::{Deserialize, Serialize};
pub const MAX_REPORT: usize = 2 * 1024 * 1024;
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub report_id: String,
    pub file_id: String,
    pub session_id: String,
    pub stage: String,
    pub parent_report_id: Option<String>,
    pub text: String,
    pub created_at: String,
    pub agent_id: String,
    pub status: String,
    pub incomplete_reason: Option<String>,
    pub file_name: String,
    pub unity_version: Option<String>,
    pub frame_count: usize,
    pub coverage: String,
}
impl Report {
    pub fn new(
        id: String,
        file_id: String,
        agent_id: String,
        parent: Option<String>,
        snapshot: &MetricsSnapshot,
    ) -> Self {
        Self {
            report_id: id.clone(),
            session_id: id,
            file_id,
            agent_id,
            stage: if parent.is_some() {
                "source"
            } else {
                "performance"
            }
            .into(),
            parent_report_id: parent,
            text: String::new(),
            created_at: chrono::Utc::now().to_rfc3339(),
            status: "running".into(),
            incomplete_reason: None,
            file_name: snapshot.meta.file_name.clone(),
            unity_version: snapshot.meta.unity_version.clone(),
            frame_count: snapshot.meta.frame_count,
            coverage: [
                ("CPU", &snapshot.cpu.main_thread_ms.quality),
                ("GC", &snapshot.gc.alloc_per_frame_bytes.quality),
                ("Draw Call", &snapshot.rendering.draw_calls.quality),
                ("SetPass", &snapshot.rendering.set_pass_calls.quality),
                ("Batches", &snapshot.rendering.batches.quality),
                ("Triangles", &snapshot.rendering.triangles.quality),
                ("Vertices", &snapshot.rendering.vertices.quality),
            ]
            .iter()
            .map(|(name, q)| {
                format!(
                    "{name} {}/{}（{}）",
                    q.valid_frames, snapshot.meta.frame_count, q.status
                )
            })
            .collect::<Vec<_>>()
            .join("；"),
        }
    }
    pub fn apply(&mut self, event: &DiagnoseEvent) {
        if self.status != "running" {
            return;
        }
        match event {
            DiagnoseEvent::Chunk { text } => {
                let mut end = text.len().min(MAX_REPORT - self.text.len());
                while !text.is_char_boundary(end) {
                    end -= 1;
                }
                self.text.push_str(&text[..end]);
                if end < text.len() {
                    self.status = "incomplete".into();
                    self.incomplete_reason = Some("正文达到 2 MiB 上限".into());
                }
            }
            DiagnoseEvent::Finished { stop_reason, .. } => {
                if stop_reason == "end_turn" && !self.text.trim().is_empty() {
                    self.status = "completed".into();
                } else {
                    self.status = "incomplete".into();
                    self.incomplete_reason = Some(if self.text.trim().is_empty() {
                        "Agent 未返回报告正文".into()
                    } else {
                        format!("Agent 停止原因：{stop_reason}")
                    });
                }
            }
            DiagnoseEvent::Cancelled => {
                self.status = "cancelled".into();
                self.incomplete_reason = Some("用户取消，正文可能不完整".into());
            }
            DiagnoseEvent::Error { message } => {
                self.status = if message.contains("REPORT_LIMIT") {
                    "incomplete"
                } else {
                    "failed"
                }
                .into();
                self.incomplete_reason = Some(message.clone());
            }
            _ => {}
        }
    }
    pub fn markdown(&self) -> String {
        format!("# {}\n\n- 录制：{}\n- Unity：{}\n- 分析帧数：{}\n- 覆盖率：{}\n- Agent：{}\n- 时间：{}\n- 状态：{}{}\n\n{}\n\n---\n\n证据边界：AI 建议需结合原始性能数据和实际源码核对；inclusive 耗时不可相加为总 CPU，源码可能与录制版本不一致。\n",if self.stage=="source"{"C# 源码定位报告"}else{"性能诊断报告"},self.file_name,self.unity_version.as_deref().unwrap_or("未知"),self.frame_count,self.coverage,self.agent_id,self.created_at,self.status,self.incomplete_reason.as_ref().map(|r|format!("（{r}）")).unwrap_or_default(),self.text)
    }
}
/// Validate identity before cloning text. Only a linked parent/child pair may be combined.
pub fn select_reports(
    store: &std::collections::HashMap<String, Report>,
    file_id: &str,
    ids: &[String],
) -> Result<Vec<Report>, String> {
    if ids.is_empty() || ids.len() > 2 {
        return Err("请选择一到两份报告".into());
    }
    let mut reports = Vec::new();
    for id in ids {
        let report = store
            .get(id)
            .filter(|r| r.file_id == file_id && !r.text.is_empty())
            .ok_or("报告不存在、为空或不属于当前录制")?;
        reports.push(report.clone());
    }
    if reports.len() == 2
        && (reports[0].report_id == reports[1].report_id
            || !reports.iter().any(|r| {
                r.parent_report_id
                    .as_ref()
                    .is_some_and(|p| reports.iter().any(|a| &a.report_id == p))
            }))
    {
        return Err("只能合并相关的首轮与源码报告".into());
    }
    reports.sort_by(|a, b| a.created_at.cmp(&b.created_at));
    Ok(reports)
}

/// Run on a blocking task. A failed write leaves both stored reports and existing target intact.
pub fn save_reports(
    reports: &[Report],
    format: &str,
    target: &std::path::Path,
) -> Result<(), String> {
    if !["markdown", "html"].contains(&format) {
        return Err("导出格式错误".into());
    }
    let markdown = reports
        .iter()
        .map(Report::markdown)
        .collect::<Vec<_>>()
        .join("\n\n---\n\n");
    let content = if format == "html" {
        document(&markdown)
    } else {
        markdown
    };
    let parent = target.parent().ok_or("无效保存路径")?;
    let temp = parent.join(format!(".upaa-report-{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| -> std::io::Result<()> {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        f.write_all(content.as_bytes())?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(&temp, target)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result.map_err(|e| e.to_string())
}
pub fn render_markdown(text: &str) -> String {
    use pulldown_cmark::{CowStr, Event, Options, Parser, Tag, TagEnd};
    let parser = Parser::new_ext(text, Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH).map(
        |event| match event {
            Event::Html(t) | Event::InlineHtml(t) => Event::Text(t),
            Event::Start(Tag::Image { .. }) => Event::Text(CowStr::from("[图片：")),
            Event::End(TagEnd::Image) => Event::Text(CowStr::from("]")),
            Event::Start(Tag::Link {
                link_type,
                dest_url,
                title,
                id,
            }) => {
                let url = dest_url.trim().to_lowercase();
                let safe = url.starts_with("https://")
                    || url.starts_with("http://")
                    || url.starts_with('#');
                Event::Start(Tag::Link {
                    link_type,
                    dest_url: if safe { dest_url } else { CowStr::from("#") },
                    title,
                    id,
                })
            }
            other => other,
        },
    );
    let mut html = String::new();
    pulldown_cmark::html::push_html(&mut html, parser);
    html
}
pub fn document(markdown: &str) -> String {
    format!("<!doctype html><html lang=\"zh-CN\"><head><meta charset=\"utf-8\"><meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; style-src 'unsafe-inline'\"><title>Unity 性能诊断报告</title><style>body{{max-width:1000px;margin:32px auto;padding:0 24px;font:16px/1.65 system-ui,sans-serif;color:#20252a}}pre{{white-space:pre-wrap;overflow-wrap:anywhere;background:#f3f4f5;padding:16px}}table{{border-collapse:collapse;max-width:100%}}td,th{{border:1px solid #bbb;padding:6px}}p,li{{overflow-wrap:anywhere}}@media print{{body{{margin:0;max-width:none;font-size:11pt}}pre,tr{{break-inside:avoid}}a{{color:inherit}}}}</style></head><body>{}</body></html>",render_markdown(markdown))
}
