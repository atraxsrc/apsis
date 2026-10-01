// SPDX-License-Identifier: GPL-3.0-only

//! Drawing: the panel button, its popup and menu, and the window (toolbar, list, status area,
//! dialogs, the settings tabs and About). Standard libcosmic widgets; every colour is a COSMIC
//! theme role.

use std::rc::Rc;
use std::time::Instant;

use apsis_core::job::{JobKind, JobState};
use apsis_core::status::{DISK_CRITICAL, DISK_LOW};
use apsis_core::{DiskUsage, Severity};
use cosmic::applet::{menu_button, padded_control};
use cosmic::iced::widget::svg as iced_svg;
use cosmic::iced::widget::text as iced_text;
use cosmic::iced::{Alignment, Color, Length};
use cosmic::prelude::*;
use cosmic::widget::text::{body, caption, heading, title4};
use cosmic::widget::{self, icon, settings};
use cosmic::{Theme, theme};

use super::{
    AppModel, CliError, Dialog, Listing, Message, Page, RowItem, SettingsLoad, Status,
    error_summary,
};
use crate::settings_view::{MAX_REMIND_DAYS, Row, Section, SettingsView};
use crate::status::{DiskStrip, StatusView};
use crate::{fl, fmt};

/// Width of the snapshot list's date column.
const DATE_WIDTH: f32 = 150.0;
/// Width of the `+`/`-` in front of a filter, so the patterns line up.
const SIGN_WIDTH: f32 = 12.0;
/// Height of the disk and progress bars.
const BAR_GIRTH: f32 = 8.0;

impl AppModel {
    /// The panel button: the icon (in the warning or destructive colour when the reminder is
    /// due or the disk is low), the optional `12h · 62%` label beside it on a horizontal
    /// panel, and a tooltip with the last snapshot and the disk. Left click opens the popup,
    /// right click the menu.
    pub(super) fn panel_button(&self) -> Element<'_, Message> {
        let status = self.status_view();
        let severity = status.severity();
        let button = if self.config.show_label && self.core.applet.is_horizontal() {
            self.label_button(&status)
        } else if severity == Severity::None {
            self.core.applet.icon_button_from_handle(self.icon.clone())
        } else {
            self.core
                .applet
                .button_from_element(self.panel_icon(severity), true)
        }
        .on_press(Message::TogglePopup);
        let button = widget::mouse_area(button).on_right_release(Message::ToggleMenu);
        self.core
            .applet
            .applet_tooltip(
                button,
                status.tooltip(),
                self.popup.is_some() || self.menu.is_some(),
                Message::Surface,
                None,
            )
            .into()
    }

    /// The panel icon, coloured by the theme's warning or destructive role.
    fn panel_icon(&self, severity: Severity) -> icon::Icon {
        let (width, height) = self.core.applet.suggested_size(true);
        let class: fn(&Theme) -> iced_svg::Style = match severity {
            Severity::None => plain_svg,
            Severity::Warning => warning_svg,
            Severity::Critical => critical_svg,
        };
        icon::icon(self.icon.clone())
            .class(theme::Svg::Custom(Rc::new(class)))
            .width(Length::Fixed(f32::from(width)))
            .height(Length::Fixed(f32::from(height)))
    }

    /// The panel button with the label beside the icon. Only the icon takes a warning colour.
    fn label_button(&self, status: &StatusView) -> widget::Button<'_, Message> {
        let (_, height) = self.core.applet.suggested_size(true);
        let (major, minor) = self.core.applet.suggested_padding(true);
        let content = widget::row::with_children(vec![
            self.panel_icon(status.severity()).into(),
            body(status.label()).into(),
        ])
        .spacing(6)
        .align_y(Alignment::Center);
        widget::button::custom(content)
            .padding([0, major])
            .height(Length::Fixed(f32::from(height + 2 * minor)))
            .class(theme::Button::AppletIcon)
    }

    /// The right-click menu: a standard COSMIC applet menu.
    pub(super) fn menu_view(&self) -> Element<'_, Message> {
        let content = widget::column::with_children(vec![
            menu_button(body(fl!("menu-open")))
                .on_press(Message::OpenWindow(None))
                .into(),
            menu_button(body(fl!("menu-refresh")))
                .on_press(Message::MenuRefresh)
                .into(),
            menu_button(body(fl!("menu-settings")))
                .on_press(Message::OpenWindow(Some("--settings")))
                .into(),
            menu_button(body(fl!("menu-about")))
                .on_press(Message::OpenWindow(Some("--about")))
                .into(),
            padded_control(widget::divider::horizontal::default()).into(),
            menu_button(body(fl!("menu-remove-or-move")))
                .on_press(Message::MenuPanelSettings)
                .into(),
            menu_button(body(fl!("menu-close")))
                .on_press(Message::MenuClose)
                .into(),
        ])
        .padding([8, 0]);
        self.core
            .applet
            .popup_container(content)
            .limits(
                cosmic::iced::Limits::NONE
                    .min_width(super::MENU_WIDTH)
                    .max_width(super::MENU_WIDTH)
                    .min_height(1.0)
                    .max_height(1000.0),
            )
            .into()
    }

    /// The panel popup: read-only. The last snapshot, the backup disk, a running job, the
    /// newest few snapshots, and buttons to open the window and to list again.
    pub(super) fn popup_view(&self) -> Element<'_, Message> {
        let status = self.status_view().strip();
        let mut children: Vec<Element<'_, Message>> = vec![
            widget::row::with_children(vec![
                icon::icon(self.icon.clone()).size(24).into(),
                title4(fl!("app-title")).into(),
            ])
            .spacing(8)
            .align_y(Alignment::Center)
            .into(),
        ];
        children.extend(self.status_lines(&status));
        if let Some(job) = self.job_line() {
            children.push(job);
        } else if let Some(line) = self.result_line() {
            children.push(line);
        }
        children.push(widget::divider::horizontal::default().into());
        children.push(self.popup_snapshots());
        children.push(
            widget::row::with_children(vec![
                widget::button::suggested(fl!("open-apsis"))
                    .on_press(Message::OpenWindow(None))
                    .into(),
                widget::space::horizontal().into(),
                widget::button::standard(fl!("refresh"))
                    .on_press_maybe((!self.loading).then_some(Message::Refresh))
                    .into(),
            ])
            .align_y(Alignment::Center)
            .into(),
        );
        widget::column::with_children(children)
            .spacing(12)
            .padding(16)
            .into()
    }

    /// The popup's newest snapshots as plain rows, or what the list says when there are none.
    fn popup_snapshots(&self) -> Element<'_, Message> {
        match &self.listing {
            Listing::Loaded(list) if !list.snapshots.is_empty() => {
                let mut rows: Vec<Element<'_, Message>> = list
                    .snapshots
                    .iter()
                    .take(super::POPUP_ROWS)
                    .map(|snapshot| {
                        widget::row::with_children(vec![
                            body(fmt::when(snapshot.created))
                                .width(Length::Fixed(DATE_WIDTH - 20.0))
                                .into(),
                            body(snapshot.comment.clone().unwrap_or_default())
                                .width(Length::Fill)
                                .wrapping(iced_text::Wrapping::None)
                                .into(),
                        ])
                        .spacing(8)
                        .into()
                    })
                    .collect();
                if let Some(more) = fmt::older_count(list.snapshots.len(), super::POPUP_ROWS) {
                    rows.push(dim(fl!("overview-more", count = more.to_string())));
                }
                widget::column::with_children(rows).spacing(4).into()
            }
            _ => self.list_message(),
        }
    }

    /// The window: the list page (toolbar, list, status area), the settings, or About.
    pub(super) fn window_view(&self) -> Element<'_, Message> {
        let content = match self.page {
            Page::List => self.list_page(),
            Page::Settings => self.settings_page(),
            Page::About => self.about_page(),
        };
        widget::container(content)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    /// The header bar's buttons in the window: refresh and About.
    pub(super) fn header_buttons(&self) -> Vec<Element<'_, Message>> {
        if self.page != Page::List {
            return Vec::new();
        }
        vec![
            widget::tooltip(
                widget::button::icon(icon::from_name("view-refresh-symbolic"))
                    .on_press_maybe((!self.loading).then_some(Message::Refresh)),
                body(fl!("refresh")),
                widget::tooltip::Position::Bottom,
            )
            .into(),
            widget::tooltip(
                widget::button::icon(icon::from_name("help-about-symbolic"))
                    .on_press(Message::AboutClicked),
                body(fl!("menu-about")),
                widget::tooltip::Position::Bottom,
            )
            .into(),
        ]
    }

    /// Toolbar, list and status area.
    fn list_page(&self) -> Element<'_, Message> {
        let toolbar = widget::row::with_children(vec![
            widget::button::standard(fl!("create"))
                .leading_icon(icon::from_name("list-add-symbolic"))
                .on_press_maybe(self.can_create().then_some(Message::CreateClicked))
                .into(),
            widget::button::standard(fl!("delete"))
                .leading_icon(icon::from_name("edit-delete-symbolic"))
                .on_press_maybe(self.can_delete().then_some(Message::DeleteClicked))
                .into(),
            widget::button::standard(fl!("settings"))
                .leading_icon(icon::from_name("emblem-system-symbolic"))
                .on_press(Message::SettingsClicked)
                .into(),
        ])
        .spacing(8);
        widget::column::with_children(vec![
            toolbar.into(),
            self.list(),
            widget::divider::horizontal::default().into(),
            self.status_area(),
        ])
        .spacing(12)
        .padding([8, 16, 16, 16])
        .into()
    }

    /// The snapshot list, with its column heads; or why there's none.
    fn list(&self) -> Element<'_, Message> {
        let rows = self.rows();
        if rows.is_empty() {
            return widget::container(self.list_message())
                .width(Length::Fill)
                .height(Length::Fill)
                .center_x(Length::Fill)
                .center_y(Length::Fill)
                .into();
        }
        let heads = widget::row::with_children(vec![
            heading(fl!("column-snapshot"))
                .width(Length::Fixed(DATE_WIDTH))
                .into(),
            heading(fl!("column-comment")).width(Length::Fill).into(),
        ])
        .padding([0, 16]);
        let mut column = widget::list_column();
        for (index, row) in rows.iter().enumerate() {
            let (date, comment): (Element<'_, Message>, Element<'_, Message>) = match row {
                RowItem::Snapshot(snapshot) => (
                    body(fmt::when(snapshot.created)).into(),
                    body(snapshot.comment.clone().unwrap_or_default())
                        .wrapping(iced_text::Wrapping::None)
                        .into(),
                ),
                RowItem::Leftover(name) => (
                    dim(apsis_core::parse_snapshot_name(name)
                        .map(fmt::when)
                        .unwrap_or_else(|| (*name).to_owned())),
                    dim(fl!("leftover-row")),
                ),
            };
            let line = widget::row::with_children(vec![
                widget::container(date)
                    .width(Length::Fixed(DATE_WIDTH))
                    .into(),
                widget::container(comment).width(Length::Fill).into(),
            ])
            .align_y(Alignment::Center);
            column = column.add(
                widget::list::button(line)
                    .selected(self.selection.contains(row.name()))
                    .on_press(Message::RowClicked(index)),
            );
        }
        widget::column::with_children(vec![
            heads.into(),
            widget::scrollable(column).height(Length::Fill).into(),
        ])
        .spacing(4)
        .height(Length::Fill)
        .into()
    }

    /// What the list area says without rows: loading, no helper, no device, empty, an error.
    fn list_message(&self) -> Element<'_, Message> {
        let lines: Vec<Element<'_, Message>> = match &self.listing {
            Listing::NotLoaded if self.helper_busy => vec![body(fl!("busy-background")).into()],
            Listing::NotLoaded => vec![body(fl!("loading")).into()],
            Listing::Loaded(list) if list.device.is_none() => {
                let mut lines = vec![body(fl!("no-device")).into()];
                if self.mode == super::Mode::Window {
                    lines.push(
                        widget::button::standard(fl!("choose-disk"))
                            .on_press(Message::SettingsClicked)
                            .into(),
                    );
                }
                lines
            }
            Listing::Loaded(_) => vec![body(fl!("empty")).into()],
            Listing::Failed(error) => {
                let mut lines = vec![
                    widget::text(error_summary(error, self.known_uuid.as_deref()))
                        .class(theme::Text::Custom(error_text))
                        .into(),
                ];
                if !matches!(error, CliError::NoHelper) {
                    lines.push(
                        widget::button::standard(fl!("retry"))
                            .on_press_maybe((!self.loading).then_some(Message::Refresh))
                            .into(),
                    );
                }
                lines
            }
        };
        widget::column::with_children(lines)
            .spacing(12)
            .align_x(Alignment::Center)
            .into()
    }

    /// The bottom of the window: last snapshot and backup disk; while a job runs, its line,
    /// bar and Stop; after one, how it went; the list's warnings.
    fn status_area(&self) -> Element<'_, Message> {
        let strip = self.status_view().strip();
        let mut children = self.status_lines(&strip);
        if let Some(job) = self.job_line() {
            children.push(job);
        } else if let Some(line) = self.result_line() {
            children.push(line);
        }
        if let Listing::Loaded(list) = &self.listing {
            children.extend(list.warnings.iter().map(|warning| {
                widget::text(warning.clone())
                    .class(theme::Text::Custom(warning_text))
                    .into()
            }));
        }
        widget::column::with_children(children).spacing(8).into()
    }

    /// `Last snapshot  12 hours ago` and `Backup disk  sda1  372G / 596G · 62% used · 224G
    /// free` with its bar.
    fn status_lines(&self, strip: &crate::status::Strip) -> Vec<Element<'_, Message>> {
        let label = |text: String| -> Element<'_, Message> {
            heading(text).width(Length::Fixed(110.0)).into()
        };
        let mut last = vec![label(fl!("last-snapshot")), body(strip.last.clone()).into()];
        if let Some(note) = &strip.last_note {
            last.push(
                widget::text(note.clone())
                    .class(theme::Text::Custom(warning_text))
                    .into(),
            );
        }
        let mut disk = vec![label(fl!("backup-disk"))];
        let mut lines = vec![widget::row::with_children(last).spacing(8).into()];
        match &strip.disk {
            DiskStrip::Mounted {
                device,
                size,
                usage,
                used_free,
                note,
            } => {
                disk.push(body(format!("{device}  {size} · {used_free}")).into());
                if let Some((text, severity)) = note {
                    let class: fn(&Theme) -> iced_text::Style = match severity {
                        Severity::Critical => error_text,
                        _ => warning_text,
                    };
                    disk.push(
                        widget::text(text.clone())
                            .class(theme::Text::Custom(class))
                            .into(),
                    );
                }
                lines.push(widget::row::with_children(disk).spacing(8).into());
                lines.push(disk_bar(*usage));
            }
            DiskStrip::Text(text) => {
                disk.push(body(text.clone()).into());
                lines.push(widget::row::with_children(disk).spacing(8).into());
            }
        }
        lines
    }

    /// The running job's line and bar (this window's or another's), with Stop for a create.
    fn job_line(&self) -> Option<Element<'_, Message>> {
        let own = self.running.is_some();
        let job = self.active_job();
        if !own && job.is_none() {
            return None;
        }
        let kind = job
            .map(|j| j.kind)
            .or(self.running.as_ref().map(super::Operation::kind))?;
        let stopping = job.is_some_and(|j| j.state == JobState::Stopping);
        let (text, fraction) = if stopping {
            (fl!("stopping"), None)
        } else {
            match kind {
                JobKind::Create => self.create_progress(),
                _ => (self.delete_progress(), None),
            }
        };
        let bar: Element<'_, Message> = match fraction {
            Some(fraction) => widget::determinate_linear(fraction)
                .width(Length::Fill)
                .girth(BAR_GIRTH)
                .into(),
            None => widget::indeterminate_linear()
                .width(Length::Fill)
                .girth(BAR_GIRTH)
                .into(),
        };
        let mut row = vec![body(text).width(Length::Fill).into()];
        if self.mode == super::Mode::Window && kind == JobKind::Create && !stopping {
            row.push(
                widget::button::destructive(fl!("stop"))
                    .on_press_maybe(self.stoppable().map(|_| Message::StopClicked))
                    .into(),
            );
        }
        Some(
            widget::column::with_children(vec![
                widget::row::with_children(row)
                    .spacing(8)
                    .align_y(Alignment::Center)
                    .into(),
                bar,
            ])
            .spacing(6)
            .into(),
        )
    }

    /// `Creating snapshot · 58% · 3 min left` and how far, or the elapsed time while there's no
    /// number.
    fn create_progress(&self) -> (String, Option<f32>) {
        self.create_progress_at(Instant::now())
    }

    pub(super) fn create_progress_at(&self, now: Instant) -> (String, Option<f32>) {
        let label = fl!("progress-creating");
        match &self.progress {
            Some(progress) if progress.has_estimate() => {
                let percent = progress.percent.unwrap_or(0.0);
                let text = match progress.eta_seconds {
                    Some(eta) => fl!(
                        "progress-percent-left",
                        label = label,
                        percent = fmt::percent(percent),
                        time = fmt::duration(eta)
                    ),
                    None => fl!(
                        "progress-percent",
                        label = label,
                        percent = fmt::percent(percent)
                    ),
                };
                #[allow(clippy::cast_possible_truncation, reason = "0 to 1")]
                (text, Some((percent / 100.0) as f32))
            }
            _ => match self.run_started {
                Some(started) => (
                    fl!(
                        "progress-working",
                        label = label,
                        elapsed = fmt::duration(now.saturating_duration_since(started).as_secs())
                    ),
                    None,
                ),
                None => (label, None),
            },
        }
    }

    /// `Deleting 09-27 09:12…` or, for this window's delete of several, `Deleting 2 of 4:
    /// 09-27 09:12…`: the step is where the helper's `JobChanged` says it is (its `snapshot`).
    pub(super) fn delete_progress(&self) -> String {
        let current = self
            .job
            .as_ref()
            .map(|j| j.snapshot.as_str())
            .filter(|s| !s.is_empty());
        match &self.running {
            Some(super::Operation::DeleteMany(names)) => {
                let step = current
                    .and_then(|now| names.iter().position(|n| n == now))
                    .unwrap_or(0);
                fl!(
                    "deleting-many",
                    step = (step + 1).to_string(),
                    count = names.len().to_string(),
                    name = names
                        .get(step)
                        .map(|n| self.snapshot_label(n))
                        .unwrap_or_default()
                )
            }
            Some(super::Operation::Delete(name)) => {
                fl!("deleting", name = self.snapshot_label(name))
            }
            _ => fl!(
                "deleting",
                name = current.map(|n| self.snapshot_label(n)).unwrap_or_default()
            ),
        }
    }

    /// How the last job went: dimmed, or in the error colour with the helper's words in a
    /// tooltip.
    fn result_line(&self) -> Option<Element<'_, Message>> {
        match self.status.as_ref()? {
            Status::Info(text) => Some(dim(text.clone())),
            Status::Error(text, details) => {
                let line: Element<'_, Message> = widget::text(text.clone())
                    .class(theme::Text::Custom(error_text))
                    .into();
                Some(match details {
                    Some(details) => {
                        widget::tooltip(line, body(details.clone()), widget::tooltip::Position::Top)
                            .into()
                    }
                    None => line,
                })
            }
        }
    }

    /// The open dialog, if any.
    pub(super) fn dialog_view(&self) -> Option<Element<'_, Message>> {
        let dialog = self.dialog.as_ref()?;
        let cancel = widget::button::standard(fl!("cancel")).on_press(Message::DialogCancel);
        Some(match dialog {
            Dialog::Create { comment, error } => {
                let mut controls: Vec<Element<'_, Message>> = vec![
                    widget::text_input(fl!("comment-placeholder"), comment.as_str())
                        .label(fl!("comment"))
                        .on_input(Message::DialogText)
                        .on_submit(|_| Message::DialogConfirm)
                        .into(),
                ];
                if let SettingsLoad::Ready(view) = &self.settings {
                    controls.push(dim(SettingsView::includes_line(&view.edited)));
                }
                if let Some(error) = error {
                    controls.push(
                        widget::text(error.clone())
                            .class(theme::Text::Custom(error_text))
                            .into(),
                    );
                }
                widget::dialog()
                    .title(fl!("create-title"))
                    .control(widget::column::with_children(controls).spacing(8))
                    .primary_action(
                        widget::button::suggested(fl!("create")).on_press(Message::DialogConfirm),
                    )
                    .secondary_action(cancel)
                    .into()
            }
            Dialog::Delete { names } => {
                let labels: Vec<String> = names.iter().map(|n| self.snapshot_label(n)).collect();
                let title = if names.len() == 1 {
                    fl!("delete-title-one")
                } else {
                    fl!("delete-title-many", count = names.len().to_string())
                };
                widget::dialog()
                    .title(title)
                    .body(format!("{}\n\n{}", labels.join("\n"), fl!("delete-body")))
                    .primary_action(
                        widget::button::destructive(fl!("delete")).on_press(Message::DialogConfirm),
                    )
                    .secondary_action(cancel)
                    .into()
            }
            Dialog::Stop { .. } => widget::dialog()
                .title(fl!("stop-title"))
                .body(fl!("stop-body"))
                .primary_action(
                    widget::button::destructive(fl!("stop")).on_press(Message::DialogConfirm),
                )
                .secondary_action(
                    widget::button::standard(fl!("keep-going")).on_press(Message::DialogCancel),
                )
                .into(),
            Dialog::AddPattern { text, error } => {
                let mut controls: Vec<Element<'_, Message>> = vec![
                    widget::text_input(fl!("pattern-placeholder"), text.as_str())
                        .label(fl!("pattern"))
                        .on_input(Message::DialogText)
                        .on_submit(|_| Message::DialogConfirm)
                        .into(),
                    dim(fl!("pattern-help")),
                ];
                if let Some(error) = error {
                    controls.push(
                        widget::text(error.clone())
                            .class(theme::Text::Custom(error_text))
                            .into(),
                    );
                }
                widget::dialog()
                    .title(fl!("add-pattern"))
                    .control(widget::column::with_children(controls).spacing(8))
                    .primary_action(
                        widget::button::suggested(fl!("add")).on_press(Message::DialogConfirm),
                    )
                    .secondary_action(cancel)
                    .into()
            }
            Dialog::Unsaved => widget::dialog()
                .title(fl!("unsaved-title"))
                .body(fl!("unsaved-body"))
                .primary_action(
                    widget::button::suggested(fl!("save")).on_press(Message::DialogConfirm),
                )
                .secondary_action(cancel)
                .tertiary_action(
                    widget::button::text(fl!("discard")).on_press(Message::DialogDiscard),
                )
                .into(),
        })
    }

    /// Settings: a back button, Cancel and Save, the tabs, and the active tab.
    fn settings_page(&self) -> Element<'_, Message> {
        let dirty = matches!(&self.settings, SettingsLoad::Ready(v) if v.dirty());
        let busy = self.saving_settings || self.running.is_some() || self.loading;
        let bar = widget::row::with_children(vec![
            widget::button::icon(icon::from_name("go-previous-symbolic"))
                .on_press(Message::Back)
                .into(),
            title4(fl!("settings")).into(),
            widget::space::horizontal().into(),
            widget::button::standard(fl!("cancel"))
                .on_press_maybe((dirty && !busy).then_some(Message::DialogDiscard))
                .into(),
            widget::button::suggested(fl!("save"))
                .on_press_maybe((dirty && !busy).then_some(Message::Save))
                .into(),
        ])
        .spacing(8)
        .align_y(Alignment::Center);
        let mut children: Vec<Element<'_, Message>> = vec![
            bar.into(),
            widget::tab_bar::horizontal(&self.tabs)
                .on_activate(Message::Tab)
                .into(),
        ];
        let body: Element<'_, Message> = match &self.settings {
            SettingsLoad::NotLoaded | SettingsLoad::Loading => body(fl!("settings-reading")).into(),
            SettingsLoad::Failed(reason) => widget::column::with_children(vec![
                widget::text(fl!("settings-read-failed"))
                    .class(theme::Text::Custom(error_text))
                    .into(),
                body(reason.clone()).into(),
            ])
            .spacing(4)
            .into(),
            SettingsLoad::Ready(view) => {
                let mut tab: Vec<Element<'_, Message>> = Vec::new();
                if view.imported() {
                    tab.push(notes(&view.info.notes));
                }
                tab.push(match self.active_section() {
                    Section::Location => location_tab(view),
                    Section::Include => include_tab(view),
                    Section::Filters => filters_tab(view),
                    Section::Misc => self.misc_tab(view),
                });
                widget::column::with_children(tab).spacing(12).into()
            }
        };
        children.push(widget::scrollable(body).height(Length::Fill).into());
        if let Some(line) = self.result_line() {
            children.push(line);
        }
        widget::column::with_children(children)
            .spacing(12)
            .padding([8, 16, 16, 16])
            .into()
    }

    /// The settings tab that's open.
    pub(super) fn active_section(&self) -> Section {
        self.tabs
            .data::<Section>(self.tabs.active())
            .copied()
            .unwrap_or(Section::Location)
    }

    /// Misc: the reminder and the panel label (Apsis's own, saved at once).
    fn misc_tab(&self, view: &SettingsView) -> Element<'_, Message> {
        let days = view.backend.remind_days;
        settings::section()
            .add(
                settings::item::builder(fl!("remind-title"))
                    .description(fl!("remind-description"))
                    .control(widget::spin_button(
                        if days == 0 {
                            fl!("remind-off")
                        } else {
                            fl!("remind-days", days = days.to_string())
                        },
                        fl!("remind-title"),
                        days,
                        1,
                        0,
                        MAX_REMIND_DAYS,
                        Message::RemindDays,
                    )),
            )
            .add(
                settings::item::builder(fl!("label-title"))
                    .description(fl!("label-description"))
                    .toggler(view.backend.show_label, Message::ShowLabel),
            )
            .into()
    }

    /// About: the app's name, version, license and link.
    fn about_page(&self) -> Element<'_, Message> {
        widget::column::with_children(vec![
            widget::row::with_children(vec![
                widget::button::icon(icon::from_name("go-previous-symbolic"))
                    .on_press(Message::Back)
                    .into(),
            ])
            .into(),
            widget::about(&self.about, |url| Message::OpenUrl(url.to_owned())),
        ])
        .spacing(12)
        .padding([8, 16, 16, 16])
        .into()
    }
}

/// Location: the disks that can hold snapshots (radio), and the others dimmed with why not.
fn location_tab(view: &SettingsView) -> Element<'_, Message> {
    let candidates = view.candidates();
    let selected = candidates
        .iter()
        .position(|d| d.uuid == view.edited.backup_device_uuid);
    let mut section = settings::section().title(fl!("location-title"));
    for (index, device) in candidates.iter().enumerate() {
        let mut description = format!("{} · {}", device.fstype, fmt::size(device.size));
        if !device.label.is_empty() {
            description.push_str(&format!(" · {}", device.label));
        }
        section = section.add(
            settings::item::builder(device.path())
                .description(description)
                .radio(index, selected, Message::PickDevice),
        );
    }
    if selected.is_none() && !view.edited.backup_device_uuid.is_empty() {
        section = section.add(
            settings::item::builder(fl!(
                "device-away",
                id = fmt::short_uuid(&view.edited.backup_device_uuid)
            ))
            .description(fl!("device-away-description"))
            .control(body(String::new())),
        );
    }
    let others: Vec<String> = view
        .info
        .devices
        .iter()
        .filter(|d| d.is_linux() && !d.selectable())
        .map(|d| {
            fl!(
                "device-unusable",
                path = d.path(),
                fstype = d.fstype.clone()
            )
        })
        .collect();
    let mut children = vec![body(fl!("location-help")).into(), section.into()];
    children.extend(others.into_iter().map(dim));
    children.push(dim(fl!("location-note")));
    widget::column::with_children(children).spacing(12).into()
}

/// Include: `/root` and `/home`, with what they mean.
fn include_tab(view: &SettingsView) -> Element<'_, Message> {
    widget::column::with_children(vec![
        body(fl!("include-help")).into(),
        settings::section()
            .add(
                settings::item::builder("/root")
                    .description(fl!("include-root"))
                    .checkbox(view.edited.include_root, |on| {
                        Message::Include(Row::IncludeRoot, on)
                    }),
            )
            .add(
                settings::item::builder("/home")
                    .description(fl!("include-home"))
                    .checkbox(view.edited.include_home, |on| {
                        Message::Include(Row::IncludeHome, on)
                    }),
            )
            .into(),
        dim(fl!("include-note")),
    ])
    .spacing(12)
    .into()
}

/// Filters: the `+`/`-` list (click selects, the checkbox keeps or leaves out), the buttons,
/// and how the list is read.
fn filters_tab(view: &SettingsView) -> Element<'_, Message> {
    let selected = match view.current() {
        Row::Filter(index) => Some(index),
        _ => None,
    };
    let list: Element<'_, Message> = if view.edited.filters.is_empty() {
        dim(fl!("filters-none"))
    } else {
        let heads = widget::row::with_children(vec![
            heading(fl!("filter-keep")).into(),
            heading(fl!("pattern")).width(Length::Fill).into(),
        ])
        .spacing(16)
        .padding([0, 16]);
        let mut column = widget::list_column();
        for (index, filter) in view.edited.filters.iter().enumerate() {
            let include = view.is_include(index);
            let pattern = filter
                .strip_prefix("+ ")
                .or_else(|| filter.strip_prefix("- "))
                .unwrap_or(filter);
            // Checked keeps the path (`+`), unchecked leaves it out (`-`); the sign stays in
            // front of the pattern, in the accent colour, as the config writes it.
            let keep =
                widget::checkbox(include).on_toggle(move |on| Message::FilterSign(index, on));
            let line = widget::row::with_children(vec![
                keep.into(),
                widget::container(
                    widget::text(if include { "+" } else { "-" }).class(theme::Text::Accent),
                )
                .width(Length::Fixed(SIGN_WIDTH))
                .center_x(Length::Fixed(SIGN_WIDTH))
                .into(),
                body(pattern.to_owned())
                    .width(Length::Fill)
                    .wrapping(iced_text::Wrapping::WordOrGlyph)
                    .into(),
            ])
            .spacing(12)
            .align_y(Alignment::Center);
            column = column.add(
                widget::list::button(line)
                    .selected(selected == Some(index))
                    .on_press(Message::FilterClicked(index)),
            );
        }
        widget::column::with_children(vec![heads.into(), column.into()])
            .spacing(4)
            .into()
    };
    let last = view.edited.filters.len().saturating_sub(1);
    let buttons = widget::flex_row(vec![
        widget::button::standard(fl!("add-folder"))
            .on_press(Message::AddFolder)
            .into(),
        widget::button::standard(fl!("add-file"))
            .on_press(Message::AddFile)
            .into(),
        widget::button::standard(fl!("add-pattern"))
            .on_press(Message::AddPattern)
            .into(),
        widget::button::standard(fl!("remove"))
            .on_press_maybe(selected.map(|_| Message::RemoveFilter))
            .into(),
        widget::button::standard(fl!("move-up"))
            .on_press_maybe(
                selected
                    .filter(|&i| i > 0)
                    .map(|_| Message::MoveFilter(true)),
            )
            .into(),
        widget::button::standard(fl!("move-down"))
            .on_press_maybe(
                selected
                    .filter(|&i| i < last)
                    .map(|_| Message::MoveFilter(false)),
            )
            .into(),
    ])
    .row_spacing(8)
    .column_spacing(8);
    widget::column::with_children(vec![list, buttons.into(), dim(fl!("filters-help"))])
        .spacing(12)
        .into()
}

/// What a conversion or import did, while it isn't saved yet.
fn notes(lines: &[String]) -> Element<'_, Message> {
    widget::container(
        widget::column::with_children(
            lines
                .iter()
                .map(|line| caption(line.trim().to_owned()).into())
                .collect::<Vec<_>>(),
        )
        .spacing(2),
    )
    .padding(12)
    .class(theme::Container::Card)
    .width(Length::Fill)
    .into()
}

/// Secondary text: the theme's text colour, faded.
fn dim<'a>(text: String) -> Element<'a, Message> {
    widget::text(text)
        .class(theme::Text::Custom(dim_text))
        .into()
}

/// The backup disk's bar: accent, the warning colour under 10% free, destructive under 5%.
fn disk_bar<'a>(usage: DiskUsage) -> Element<'a, Message> {
    let used = usage.used as f64 / (usage.used + usage.free).max(1) as f64;
    #[allow(clippy::cast_possible_truncation, reason = "0 to 1")]
    let mut bar = widget::determinate_linear(used as f32)
        .width(Length::Fill)
        .girth(BAR_GIRTH);
    let free = usage.free_fraction();
    let active = theme::active();
    let cosmic = active.cosmic();
    if free < DISK_CRITICAL {
        bar = bar.bar_color(Color::from(cosmic.destructive_color()));
    } else if free < DISK_LOW {
        bar = bar.bar_color(Color::from(cosmic.warning_color()));
    }
    bar.into()
}

/// The panel icon as the panel draws it: the normal text colour.
fn plain_svg(theme: &Theme) -> iced_svg::Style {
    iced_svg::Style {
        color: Some(Color::from(theme.cosmic().background(theme.transparent).on)),
    }
}

/// The panel icon while the reminder is due or the disk is low.
fn warning_svg(theme: &Theme) -> iced_svg::Style {
    iced_svg::Style {
        color: Some(theme.cosmic().warning_text_color().into()),
    }
}

/// The panel icon while the disk is nearly full.
fn critical_svg(theme: &Theme) -> iced_svg::Style {
    iced_svg::Style {
        color: Some(theme.cosmic().destructive_text_color().into()),
    }
}

/// Secondary text: the normal text colour, faded.
fn dim_text(theme: &Theme) -> iced_text::Style {
    let on = theme.cosmic().background(theme.transparent).on;
    text_style(theme, Color::from(on).scale_alpha(0.7))
}

/// The reminder's note, low space, and the list's warnings.
fn warning_text(theme: &Theme) -> iced_text::Style {
    text_style(theme, theme.cosmic().warning_text_color().into())
}

/// Failures, and space nearly gone.
fn error_text(theme: &Theme) -> iced_text::Style {
    text_style(theme, theme.cosmic().destructive_text_color().into())
}

/// A text style in `color`, with the theme's usual selection colours.
fn text_style(theme: &Theme, color: Color) -> iced_text::Style {
    let cosmic = theme.cosmic();
    iced_text::Style {
        color: Some(color),
        selected_fill: cosmic.accent.base.into(),
        selected_text_color: Some(cosmic.on_accent_color().into()),
    }
}
