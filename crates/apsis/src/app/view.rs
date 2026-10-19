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
    AppModel, Dialog, Listing, Message, Page, RowItem, SettingsLoad, Status, Tone, error_summary,
};
use crate::settings_view::{MAX_REMIND_DAYS, Row, Section, SettingsView};
use crate::status::{PopupStatus, StatusView};
use crate::{fl, fmt};

/// Width of the `+`/`-` in front of a filter, so the patterns line up.
const SIGN_WIDTH: f32 = 12.0;
/// Height of the disk and progress bars.
const BAR_GIRTH: f32 = 8.0;
/// What the restore dialog needs besides its scrolling body: its padding, title, the muted
/// line, the gaps and the button row, plus a margin to the window's edges. The
/// body scrolls within the window's height minus this, so the buttons always show.
const DIALOG_CHROME: f32 = 216.0;

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

    /// The panel popup: read-only. One status line (the last snapshot's age, or the running
    /// job), the bar the window has under it (the backup disk's use, or the job's progress),
    /// any error, and a button to open the window.
    pub(super) fn popup_view(&self) -> Element<'_, Message> {
        let chosen = !matches!(&self.listing, Listing::Loaded(list) if list.device.is_none());
        self.popup_with(self.status_view().popup(chosen))
    }

    /// The popup for `status` (used while no job runs).
    pub(super) fn popup_with(&self, status: PopupStatus) -> Element<'_, Message> {
        let mut children: Vec<Element<'_, Message>> = vec![
            widget::row::with_children(vec![
                icon::icon(self.icon.clone()).size(24).into(),
                title4(fl!("app-title")).into(),
            ])
            .spacing(8)
            .align_y(Alignment::Center)
            .into(),
        ];
        if let Some(job) = self.job_status() {
            children.push(body(self.popup_job_line(&job)).into());
            let fraction = if job.kind == JobKind::Create && !job.stopping {
                #[allow(clippy::cast_possible_truncation, reason = "0 to 1")]
                self.progress_percent()
                    .map(|p| (p / 100.0).clamp(0.0, 1.0) as f32)
            } else {
                job.fraction
            };
            children.push(job_bar(fraction));
        } else {
            let line = if status.last.is_empty() {
                fl!("loading")
            } else {
                status.last.clone()
            };
            let line = widget::text(line);
            children.push(if status.overdue {
                line.class(theme::Text::Custom(warning_text)).into()
            } else {
                line.into()
            });
            match (status.usage, &status.figures) {
                (Some(usage), Some(figures)) => children.push(
                    widget::tooltip(
                        disk_bar(usage),
                        body(figures.clone()),
                        widget::tooltip::Position::Bottom,
                    )
                    .into(),
                ),
                (Some(usage), None) => children.push(disk_bar(usage)),
                (None, _) if !status.disk.is_empty() => children.push(dim(status.disk.clone())),
                (None, _) => {}
            }
        }
        children.extend(self.popup_errors());
        children.push(
            widget::button::suggested(fl!("open-apsis"))
                .on_press(Message::OpenWindow(None))
                .into(),
        );
        widget::column::with_children(children)
            .spacing(12)
            .padding(16)
            .into()
    }

    /// The popup's line for a running job: as the window's, without the time left (`Creating
    /// snapshot · 42%`, `Creating snapshot · scanning files`).
    pub(super) fn popup_job_line(&self, job: &JobStatus) -> String {
        if job.stopping {
            return fl!("stopping");
        }
        match job.kind {
            JobKind::Create => match self.progress_percent() {
                Some(percent) => fl!(
                    "progress-percent",
                    label = fl!("progress-creating"),
                    percent = fmt::percent(percent)
                ),
                None => fl!("progress-scanning-short", label = fl!("progress-creating")),
            },
            JobKind::Restore if self.ready.is_some() => fl!("popup-ready"),
            _ => job.text.clone(),
        }
    }

    /// The percent to show for a create: this window's progress, else the helper's.
    fn progress_percent(&self) -> Option<f64> {
        self.progress
            .as_ref()
            .filter(|p| p.has_estimate())
            .and_then(|p| p.percent)
            .or_else(|| {
                self.active_job()
                    .and_then(|j| j.percent)
                    .filter(|p| *p > 0.0)
            })
    }

    /// The popup's error lines: a list that failed, a job that failed, a restore that didn't
    /// end well. Lines that only say something went fine are the window's.
    fn popup_errors(&self) -> Vec<Element<'_, Message>> {
        let mut lines = Vec::new();
        if let Listing::Failed(error) = &self.listing {
            lines.push(
                widget::text(error_summary(error, self.known_uuid.as_deref()))
                    .class(theme::Text::Custom(error_text))
                    .into(),
            );
        }
        match &self.status {
            Some(Status::Error(..)) => lines.extend(self.result_line()),
            None if self.result_phrase().is_some() => lines.extend(self.restore_result_line()),
            _ => {}
        }
        lines
    }

    /// The window: the list page (toolbar, list, status area), the settings, or About.
    pub(super) fn window_view(&self) -> Element<'_, Message> {
        let content = match self.page {
            Page::List => self.list_page(),
            Page::Settings => self.settings_page(),
            Page::About => self.about_page(),
        };
        widget::toaster(
            &self.toasts,
            widget::container(content)
                .width(Length::Fill)
                .height(Length::Fill),
        )
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
            // Restore: no tooltip, no key.
            widget::button::standard(fl!("restore"))
                .leading_icon(icon::from_name("document-revert-symbolic"))
                .on_press_maybe(self.can_restore().then_some(Message::RestoreClicked))
                .into(),
            widget::button::standard(self.delete_label())
                .leading_icon(icon::from_name("edit-delete-symbolic"))
                .on_press_maybe(self.can_delete().then_some(Message::DeleteClicked))
                .into(),
            widget::button::standard(fl!("settings"))
                .leading_icon(icon::from_name("emblem-system-symbolic"))
                .on_press(Message::SettingsClicked)
                .into(),
        ])
        .spacing(8);
        let mut children: Vec<Element<'_, Message>> = vec![toolbar.into()];
        children.extend(self.banners().into_iter().map(banner));
        children.extend([
            self.list(),
            widget::divider::horizontal::default().into(),
            self.status_area(),
        ]);
        widget::column::with_children(children)
            .spacing(12)
            .padding([8, 16, 16, 16])
            .into()
    }

    /// The snapshot list: one two-line row per snapshot (the comment or the date over the
    /// date and the age), then the leftovers with their Remove button; or why there's none.
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
        let now = jiff::Zoned::now().datetime();
        let mut column = widget::list_column();
        for (index, row) in rows.iter().enumerate() {
            let Some((title, subtitle)) = self.row_text(index, now) else {
                continue;
            };
            let leftover = matches!(row, RowItem::Leftover(_));
            let title: Element<'_, Message> = if leftover {
                dim(title)
            } else {
                body(title).wrapping(iced_text::Wrapping::Word).into()
            };
            let lines = widget::column::with_children(vec![
                title,
                caption(subtitle)
                    .class(theme::Text::Custom(dim_text))
                    .wrapping(iced_text::Wrapping::Word)
                    .into(),
            ])
            .spacing(2)
            .width(Length::Fill);
            let content: Element<'_, Message> = if leftover {
                widget::row::with_children(vec![
                    lines.into(),
                    widget::button::standard(fl!("remove"))
                        .on_press_maybe(
                            self.can_create()
                                .then(|| Message::RemoveLeftover(row.name().to_owned())),
                        )
                        .into(),
                ])
                .spacing(12)
                .align_y(Alignment::Center)
                .into()
            } else {
                lines.into()
            };
            column = column.add(
                widget::list::button(content)
                    .selected(self.selection.contains(row.name()))
                    .on_press(Message::RowClicked(index)),
            );
        }
        widget::scrollable(column).height(Length::Fill).into()
    }

    /// The list area without rows: the orbit icon faded, one line, at most one button
    /// (`empty_state`).
    fn list_message(&self) -> Element<'_, Message> {
        let Some(state) = self.empty_state() else {
            return widget::space::vertical().into();
        };
        let mut lines: Vec<Element<'_, Message>> = vec![
            icon::icon(self.icon.clone())
                .size(64)
                .class(theme::Svg::Custom(Rc::new(faded_svg)))
                .into(),
        ];
        lines.push(if state.error {
            widget::text(state.line)
                .class(theme::Text::Custom(error_text))
                .into()
        } else {
            body(state.line).into()
        });
        if let Some((label, message)) = state.action {
            lines.push(widget::button::suggested(label).on_press(message).into());
        }
        widget::column::with_children(lines)
            .spacing(12)
            .align_x(Alignment::Center)
            .into()
    }

    /// The bottom of the window: a card with the backup disk (its icon, name and size, the
    /// free space on the right) and its bar, then the running job (its line, bar and Stop)
    /// or the last snapshot's age and how the last job went; the list's warnings under it.
    fn status_area(&self) -> Element<'_, Message> {
        let card = self.disk_card_at(jiff::Zoned::now().datetime());
        let drive = if self.disk_removable == Some(true) {
            "drive-removable-media-symbolic"
        } else {
            "drive-harddisk-symbolic"
        };
        let mut head: Vec<Element<'_, Message>> = vec![
            icon::from_name(drive).size(20).into(),
            body(card.title).into(),
            widget::space::horizontal().into(),
        ];
        if !card.right.is_empty() {
            head.push(
                caption(card.right)
                    .class(theme::Text::Custom(dim_text))
                    .into(),
            );
        }
        let mut lines: Vec<Element<'_, Message>> = vec![
            widget::row::with_children(head)
                .spacing(10)
                .align_y(Alignment::Center)
                .into(),
        ];
        if let Some(usage) = card.usage {
            lines.push(disk_bar(usage));
        }
        if let Some(job) = self.job_line() {
            lines.push(job);
        } else {
            if !card.last.is_empty() {
                lines.push(body(card.last).into());
            }
            if let Some(line) = self.window_result() {
                lines.push(line);
            }
        }
        let mut children: Vec<Element<'_, Message>> = vec![
            widget::container(widget::column::with_children(lines).spacing(8))
                .class(theme::Container::Card)
                .padding(12)
                .width(Length::Fill)
                .into(),
        ];
        if let Listing::Loaded(list) = &self.listing {
            children.extend(list.warnings.iter().map(|warning| {
                widget::text(warning.clone())
                    .class(theme::Text::Custom(warning_text))
                    .into()
            }));
        }
        widget::column::with_children(children).spacing(8).into()
    }

    /// The running job (this window's or another's): which, its line in the window and how
    /// far. `None` while nothing runs.
    pub(super) fn job_status(&self) -> Option<JobStatus> {
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
                JobKind::Restore => self.restore_progress(),
                _ => (self.delete_progress(), None),
            }
        };
        Some(JobStatus {
            kind,
            stopping,
            text,
            fraction,
        })
    }

    /// The running job's line and bar (this window's or another's), with Stop for a create.
    fn job_line(&self) -> Option<Element<'_, Message>> {
        let JobStatus {
            kind,
            stopping,
            text,
            fraction,
        } = self.job_status()?;
        let mut row = vec![body(text).width(Length::Fill).into()];
        // Stop for a create, and for a restore's preparation until it's ready.
        let can_stop = matches!(kind, JobKind::Create | JobKind::Restore) && self.ready.is_none();
        if self.mode == super::Mode::Window && can_stop && !stopping {
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
                job_bar(fraction),
            ])
            .spacing(6)
            .into(),
        )
    }

    /// `Creating snapshot · 58% · 3 min left` and how far, or `scanning files` and the elapsed
    /// time until rsync's file list is complete.
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
                        "progress-scanning",
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
        if self.status.is_none() {
            return self.restore_result_line();
        }
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
            Dialog::Restore {
                snapshot,
                check,
                restore_home,
                safety_snapshot,
            } => self.restore_dialog(dialog, snapshot, check, *restore_home, *safety_snapshot),
            // The reason in the error colour (as a bad pattern in Add Pattern), what to do in
            // plain text, and for a dropped plan that a fresh Restore re-measures; Close.
            Dialog::Refused { snapshot, .. } => {
                let mut lines: Vec<Element<'_, Message>> = vec![self.muted_line(snapshot)];
                for (index, line) in dialog.refusal_lines().into_iter().enumerate() {
                    lines.push(if index == 0 {
                        widget::text(line)
                            .class(theme::Text::Custom(error_text))
                            .into()
                    } else {
                        body(line).into()
                    });
                }
                widget::dialog()
                    .title(fl!("refused-title"))
                    .control(widget::column::with_children(lines).spacing(12))
                    .primary_action(
                        widget::button::standard(fl!("close")).on_press(Message::DialogCancel),
                    )
                    .into()
            }
            // 0.4.0's Stop dialog, for the restore's preparation.
            Dialog::StopRestore { .. } => widget::dialog()
                .title(fl!("stop-restore-title"))
                .body(fl!("stop-restore-body"))
                .primary_action(
                    widget::button::destructive(fl!("stop")).on_press(Message::DialogConfirm),
                )
                .secondary_action(
                    widget::button::standard(fl!("keep-going")).on_press(Message::DialogCancel),
                )
                .into(),
            // Two answers; nothing is armed until Restart now. Esc cancels.
            // The date and comment first: the last place to see which row this is (it's
            // easy to restore a safety snapshot by mistake).
            Dialog::Ready { snapshot } => widget::dialog()
                .title(fl!("ready-title"))
                .control(
                    widget::column::with_children(vec![
                        self.muted_line(snapshot),
                        body(format!(
                            "{}\n\n{}",
                            fl!("ready-lead"),
                            fl!("ready-body", date = super::date_of(snapshot))
                        ))
                        .into(),
                    ])
                    .spacing(12),
                )
                .primary_action(
                    widget::button::suggested(fl!("restart-now")).on_press(Message::RestartNow),
                )
                .secondary_action(
                    widget::button::standard(fl!("cancel-restore"))
                        .on_press(Message::CancelRestore),
                )
                .into(),
        })
    }

    /// The one restore dialog: the title, a muted line, and a body that scrolls
    /// within the window so the buttons always show. The radios are the Location tab's
    /// pattern, the checkbox the Include tab's.
    fn restore_dialog(
        &self,
        dialog: &Dialog,
        snapshot: &str,
        check: &apsis_core::restore::dialog::Dialog,
        restore_home: bool,
        safety_snapshot: bool,
    ) -> Element<'_, Message> {
        let date = super::date_of(snapshot);
        let mut body_lines: Vec<Element<'_, Message>> = vec![body(fl!("restore-body")).into()];
        if check.has_home {
            let restore_too = settings::item::builder(fl!("restore-restore-home"));
            let restore_too = if dialog.lost_for_good() {
                restore_too.description(fl!("restore-lost-for-good", date = date.as_str()))
            } else {
                restore_too
            };
            let selected = Some(usize::from(restore_home));
            body_lines.push(
                settings::section()
                    .title(fl!("restore-home-section"))
                    .add(settings::item::builder(fl!("restore-keep-home")).radio(
                        0,
                        selected,
                        Message::RestoreHome,
                    ))
                    .add(restore_too.radio(1, selected, Message::RestoreHome))
                    .into(),
            );
        }
        body_lines.push(
            settings::section()
                .add(
                    settings::item::builder(fl!("restore-safety"))
                        .checkbox(safety_snapshot, Message::RestoreSafety),
                )
                .into(),
        );
        for line in dialog.extra_lines() {
            body_lines.push(body(line).into());
        }
        body_lines.push(dim(fl!("restore-experimental")));
        let body_height = (self.window_height - DIALOG_CHROME).max(80.0);
        widget::dialog()
            .title(fl!("restore-title"))
            .control(
                widget::column::with_children(vec![
                    self.muted_line(snapshot),
                    widget::container(widget::scrollable(
                        widget::column::with_children(body_lines)
                            .spacing(12)
                            // Room for the scrollbar.
                            .padding([0, 16, 0, 0]),
                    ))
                    .max_height(body_height)
                    .into(),
                ])
                .spacing(12),
            )
            .primary_action(
                widget::button::destructive(fl!("restore")).on_press(Message::DialogConfirm),
            )
            .secondary_action(
                widget::button::standard(fl!("cancel")).on_press(Message::DialogCancel),
            )
            .into()
    }

    /// `2026-09-25 11:28 · before driver update`: the date as the list shows it, then the
    /// comment if there is one; muted, one line.
    fn muted_line(&self, snapshot: &str) -> Element<'_, Message> {
        let date = super::date_of(snapshot);
        let text = match self
            .snapshots()
            .iter()
            .find(|s| s.name == snapshot)
            .and_then(|s| s.comment.as_deref())
        {
            Some(comment) => fl!("restore-muted", date = date.as_str(), comment = comment),
            None => date,
        };
        widget::text(text)
            .class(theme::Text::Custom(dim_text))
            .wrapping(iced_text::Wrapping::None)
            .into()
    }

    /// The last restore's result: plain for done and not-started,
    /// else the colour on the phrase only (`result_phrase`), the full text in a tooltip, and
    /// Restore again after a failure.
    fn restore_result_line(&self) -> Option<Element<'_, Message>> {
        let (text, tooltip) = self.result_text()?;
        let line: Element<'_, Message> = match self.result_phrase() {
            Some(parts) => phrase_line(
                parts.before,
                parts.phrase,
                match parts.tone {
                    Tone::Warning => warning_text,
                    Tone::Error => error_text,
                },
                parts.after,
            ),
            None => body(text).width(Length::Fill).into(),
        };
        let line: Element<'_, Message> =
            widget::tooltip(line, body(tooltip), widget::tooltip::Position::Top).into();
        if !self.restore_again_available() {
            return Some(line);
        }
        Some(
            widget::row::with_children(vec![
                line,
                widget::button::standard(fl!("restore-again"))
                    .on_press(Message::RestoreAgain)
                    .into(),
            ])
            .spacing(8)
            .align_y(Alignment::Center)
            .into(),
        )
    }

    /// The window's result: how the last job went (`result_line`), or the last restore's
    /// outcome as a banner with Details (and Restore again after a failure); a restore that
    /// went fine is a plain line with Details.
    fn window_result(&self) -> Option<Element<'_, Message>> {
        if self.status.is_some() {
            return self.result_line();
        }
        let (text, _) = self.result_text()?;
        let mut row: Vec<Element<'_, Message>> = vec![
            widget::text(text)
                .width(Length::Fill)
                .wrapping(iced_text::Wrapping::Word)
                .into(),
            widget::button::standard(fl!("details"))
                .on_press(Message::ShowDetails)
                .into(),
        ];
        if self.restore_again_available() {
            row.push(
                widget::button::standard(fl!("restore-again"))
                    .on_press(Message::RestoreAgain)
                    .into(),
            );
        }
        let row = widget::row::with_children(row)
            .spacing(12)
            .align_y(Alignment::Center);
        let style: Option<fn(&Theme) -> widget::container::Style> = match self.result_tone() {
            Some(Tone::Warning) => Some(widget::warning_container),
            Some(Tone::Error) => Some(destructive_container),
            None => None,
        };
        Some(match style {
            Some(style) => widget::container(row)
                .class(theme::Container::custom(style))
                .padding(10)
                .width(Length::Fill)
                .into(),
            None => row.into(),
        })
    }

    /// The restore details drawer: a label over each line.
    pub(super) fn details_view(&self) -> Element<'_, Message> {
        let mut lines: Vec<Element<'_, Message>> = Vec::new();
        for (label, text) in self.details_lines() {
            lines.push(heading(label).into());
            lines.push(body(text).wrapping(iced_text::Wrapping::Word).into());
        }
        widget::column::with_children(lines).spacing(6).into()
    }

    /// Settings: a back button, Cancel and Save, the tabs, and the active tab. Cancel is
    /// always clickable: it drops any change and leaves the page.
    fn settings_page(&self) -> Element<'_, Message> {
        let dirty = matches!(&self.settings, SettingsLoad::Ready(v) if v.dirty());
        let busy = self.saving_settings || self.running.is_some() || self.loading;
        let mut bar: Vec<Element<'_, Message>> = vec![
            widget::button::icon(icon::from_name("go-previous-symbolic"))
                .on_press(Message::Back)
                .into(),
            title4(fl!("settings")).into(),
            widget::space::horizontal().into(),
        ];
        if let Some(hint) = self.unsaved_hint() {
            bar.push(caption(hint).class(theme::Text::Custom(dim_text)).into());
        }
        bar.extend([
            widget::button::standard(fl!("cancel"))
                .on_press(Message::DialogDiscard)
                .into(),
            widget::button::suggested(fl!("save"))
                .on_press_maybe((dirty && !busy).then_some(Message::Save))
                .into(),
        ]);
        let bar = widget::row::with_children(bar)
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

/// Location: one list. The disks that can hold snapshots (a radio and a drive icon), the
/// chosen one when it's away (its subtitle in the warning colour), and the Linux devices
/// that can't (dimmed, with why); then where snapshots go.
fn location_tab(view: &SettingsView) -> Element<'_, Message> {
    let rows = view.location_rows(&|name| {
        apsis_core::status::removable(
            std::path::Path::new(apsis_core::status::SYS_CLASS_BLOCK),
            name,
        )
    });
    let selected = rows.iter().find(|r| r.selected).and_then(|r| r.choice);
    let mut section = settings::section().title(fl!("location-title"));
    for row in rows {
        let name = if row.removable == Some(true) {
            "drive-removable-media-symbolic"
        } else {
            "drive-harddisk-symbolic"
        };
        let faded = row.choice.is_none() && !row.warning;
        let mut drive = icon::icon(icon::from_name(name).handle()).size(20);
        if faded {
            drive = drive.class(theme::Svg::Custom(Rc::new(faded_svg)));
        }
        section = match row.choice {
            Some(index) => section.add(
                settings::item::builder(row.title)
                    .description(row.subtitle)
                    .icon(drive)
                    .radio(index, selected, Message::PickDevice),
            ),
            None => {
                let subtitle = caption(row.subtitle).class(theme::Text::Custom(if row.warning {
                    warning_text
                } else {
                    dim_text
                }));
                let title: Element<'_, Message> = if faded {
                    dim(row.title)
                } else {
                    body(row.title).into()
                };
                section.add(
                    widget::row::with_children(vec![
                        drive.into(),
                        widget::column::with_children(vec![title, subtitle.into()])
                            .spacing(2)
                            .width(Length::Fill)
                            .into(),
                    ])
                    .spacing(12)
                    .align_y(Alignment::Center),
                )
            }
        };
    }
    widget::column::with_children(vec![section.into(), dim(fl!("location-note"))])
        .spacing(12)
        .into()
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
/// `before`, then `phrase` in `role`'s colour, then `after` in the normal text colour, the
/// last part wrapping instead of being cut off.
fn phrase_line<'a>(
    before: String,
    phrase: String,
    role: fn(&Theme) -> iced_text::Style,
    after: String,
) -> Element<'a, Message> {
    widget::row::with_children(vec![
        body(before).into(),
        widget::text(phrase).class(theme::Text::Custom(role)).into(),
        body(after)
            .width(Length::Fill)
            .wrapping(iced_text::Wrapping::WordOrGlyph)
            .into(),
    ])
    .spacing(4)
    .align_y(Alignment::Center)
    .into()
}

/// A warning strip in the theme's warning colours: the text, and its button if it has one.
fn banner<'a>(banner: super::Banner) -> Element<'a, Message> {
    let mut row: Vec<Element<'a, Message>> = vec![
        widget::text(banner.text)
            .width(Length::Fill)
            .wrapping(iced_text::Wrapping::Word)
            .into(),
    ];
    if let Some((label, message)) = banner.action {
        row.push(widget::button::standard(label).on_press(message).into());
    }
    widget::container(
        widget::row::with_children(row)
            .spacing(12)
            .align_y(Alignment::Center),
    )
    .class(theme::Container::custom(widget::warning_container))
    .padding(10)
    .width(Length::Fill)
    .into()
}

/// A strip in the theme's destructive colours, as the warning widget's in the warning ones.
fn destructive_container(theme: &Theme) -> widget::container::Style {
    let cosmic = theme.cosmic();
    widget::container::Style {
        icon_color: Some(cosmic.destructive.on.into()),
        text_color: Some(cosmic.destructive.on.into()),
        background: Some(cosmic::iced::Background::Color(
            cosmic.destructive_color().into(),
        )),
        border: cosmic::iced::Border {
            color: Color::TRANSPARENT,
            width: 1.0,
            radius: cosmic.corner_radii.radius_0.into(),
        },
        ..widget::container::Style::default()
    }
}

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

/// A job's bar: how far, or moving while there's no number yet.
fn job_bar<'a>(fraction: Option<f32>) -> Element<'a, Message> {
    match fraction {
        Some(fraction) => widget::determinate_linear(fraction)
            .width(Length::Fill)
            .girth(BAR_GIRTH)
            .into(),
        None => widget::indeterminate_linear()
            .width(Length::Fill)
            .girth(BAR_GIRTH)
            .into(),
    }
}

/// The panel icon as the panel draws it: the normal text colour.
fn plain_svg(theme: &Theme) -> iced_svg::Style {
    iced_svg::Style {
        color: Some(Color::from(theme.cosmic().background(theme.transparent).on)),
    }
}

/// The big icon behind an empty list: the text colour, faded.
fn faded_svg(theme: &Theme) -> iced_svg::Style {
    let on = theme.cosmic().background(theme.transparent).on;
    iced_svg::Style {
        color: Some(Color::from(on).scale_alpha(0.35)),
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

/// A running job as the window and the popup show it.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct JobStatus {
    pub kind: JobKind,
    pub stopping: bool,
    /// The window's line: `Creating snapshot · 58% · 3 min left`.
    pub text: String,
    /// How far, 0 to 1; `None` while there's no number.
    pub fraction: Option<f32>,
}
