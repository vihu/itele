//! The guide grid: which channels it lists (the Live TV group), the time
//! window it shows, the selected programme, and moving around with keys.
//!
//! Rows load their programmes lazily, as the grid reports what is on
//! screen. Times are Unix seconds; positions sent to the UI are minutes
//! from the window start.

use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;

use itele::epg::Programme;
use slint::{Model, ModelRc, VecModel};

use super::Session;
use super::catchup::replayable;
use super::timefmt::{HALF_HOUR, clock, day_label, day_time, half_hour, now, when};
use crate::ui::{GuideCell, GuideDetails, GuideRow, GuideTick, Screen};

/// Programmes fetched per row: the visible span plus room to move.
const WINDOW: i64 = 6 * 3600;
/// Rows moved by Page Up and Page Down.
const PAGE: i32 = 10;
/// Rows around the selection assumed on screen when the guide opens.
const SEED_ROWS: usize = 12;
/// Pixels per minute in the grid, as in `guide.slint`.
const PX_PER_MINUTE: f32 = 6.0;

/// The grid's state.
#[derive(Default)]
pub(super) struct Grid {
    /// Window start, on a half hour.
    start: i64,
    /// The selected time; the selected programme is the one airing then.
    cursor: i64,
    row: usize,
    rows: Rc<VecModel<GuideRow>>,
    programmes: HashMap<usize, Vec<Programme>>,
    visible: Range<usize>,
}

impl Session {
    /// Shows the guide for the Live TV group, at now, on the selected
    /// channel.
    pub(super) fn open_guide(&self) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        let now = now();
        let rows: Vec<GuideRow> = {
            let state = self.state.borrow();
            let logos = self.logos.borrow();
            (0..state.channels.row_count())
                .filter_map(|r| state.channels.row_data(r))
                .enumerate()
                .map(|(r, item)| GuideRow {
                    number: item.number,
                    name: item.name,
                    short: item.short,
                    tint: item.tint,
                    logo: state
                        .row_logos
                        .get(r)
                        .and_then(|u| logos.get(u))
                        .unwrap_or(item.logo),
                    has_logo: item.has_logo
                        || state
                            .row_logos
                            .get(r)
                            .is_some_and(|u| logos.get(u).is_some()),
                    cells: ModelRc::default(),
                })
                .collect()
        };
        {
            let mut state = self.state.borrow_mut();
            let row = usize::try_from(app.get_channel_index()).unwrap_or(0);
            state.grid = Grid {
                start: half_hour(now) - HALF_HOUR,
                cursor: now,
                row: row.min(rows.len().saturating_sub(1)),
                rows: Rc::new(VecModel::from(rows)),
                programmes: HashMap::new(),
                // The grid reports what it shows only when it scrolls; until
                // then, assume the rows around the selection.
                visible: row.saturating_sub(SEED_ROWS)..row + SEED_ROWS,
            };
            app.set_guide_rows(ModelRc::from(Rc::clone(&state.grid.rows)));
            app.set_guide_row(state.grid.row as i32);
        }
        self.show(Screen::Guide);
        app.invoke_reveal_guide_row();
        let visible = self.state.borrow().grid.visible.clone();
        self.fill_grid(visible);
        self.redraw_grid();
    }

    /// The grid reports rows `first` to `first + count` on screen.
    pub(super) fn grid_rows_visible(&self, first: i32, count: i32) {
        let first = first.max(0) as usize;
        let range = first..first + count.max(0) as usize;
        self.state.borrow_mut().grid.visible = range.clone();
        self.fill_grid(range);
        let urls: Vec<String> = {
            let state = self.state.borrow();
            let end = state.grid.visible.end.min(state.row_logos.len());
            state
                .row_logos
                .get(first.min(end)..end)
                .unwrap_or_default()
                .to_vec()
        };
        for url in urls {
            self.logos.borrow_mut().request(&url);
        }
    }

    /// Moves the selection: `dx` programmes along the row, `dy` channels.
    pub(super) fn grid_move(&self, dx: i32, dy: i32) {
        {
            let mut state = self.state.borrow_mut();
            let count = state.grid.rows.row_count();
            if count == 0 {
                return;
            }
            let grid = &mut state.grid;
            grid.row = (grid.row as i32 + dy).clamp(0, count as i32 - 1) as usize;
        }
        if dx != 0 {
            let row = self.state.borrow().grid.row;
            self.fill_grid(row..row + 1);
            let mut state = self.state.borrow_mut();
            let grid = &mut state.grid;
            let programmes = grid
                .programmes
                .get(&grid.row)
                .map(Vec::as_slice)
                .unwrap_or_default();
            grid.cursor = step(programmes, grid.cursor, dx);
        }
        self.follow_cursor();
    }

    pub(super) fn grid_page(&self, direction: i32) {
        self.grid_move(0, direction.signum() * PAGE);
    }

    /// Jumps back to now.
    pub(super) fn grid_now(&self) {
        let now = now();
        {
            let mut state = self.state.borrow_mut();
            state.grid.cursor = now;
            state.grid.start = half_hour(now) - HALF_HOUR;
            state.grid.programmes.clear();
        }
        self.refill_grid();
    }

    /// Moves the window two hours earlier or later.
    pub(super) fn grid_shift(&self, direction: i32) {
        {
            let mut state = self.state.borrow_mut();
            let shift = i64::from(direction.signum()) * 4 * HALF_HOUR;
            state.grid.start += shift;
            state.grid.cursor = state.grid.start + HALF_HOUR;
            state.grid.programmes.clear();
        }
        self.refill_grid();
    }

    pub(super) fn grid_cell_clicked(&self, row: i32, cell: i32) {
        {
            let mut state = self.state.borrow_mut();
            let grid = &mut state.grid;
            let (Ok(row), Ok(cell)) = (usize::try_from(row), usize::try_from(cell)) else {
                return;
            };
            // Cells are drawn one per programme, in order.
            let Some(programme) = grid.programmes.get(&row).and_then(|p| p.get(cell)) else {
                return;
            };
            grid.row = row;
            grid.cursor = programme.start.max(grid.start);
        }
        self.follow_cursor();
    }

    /// Enter: watches the channel when its selected programme is on now,
    /// or replays the programme when it ended and the channel keeps
    /// catch-up.
    pub(super) fn grid_enter(&self) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        let (row, programme) = {
            let state = self.state.borrow();
            let grid = &state.grid;
            let programme = grid
                .programmes
                .get(&grid.row)
                .and_then(|p| at(p, grid.cursor))
                .cloned();
            (grid.row, programme)
        };
        let Some(programme) = programme else {
            return;
        };
        let now = now();
        app.set_channel_index(row as i32);
        if programme.start <= now && now < programme.stop {
            self.refresh_programme();
            self.watch();
        } else if programme.stop <= now {
            self.replay_selected(programme);
        }
    }

    /// Shows a downloaded logo on the guide row that uses it.
    pub(super) fn show_grid_logo(&self, row: usize, image: &slint::Image) {
        let state = self.state.borrow();
        if let Some(mut item) = state.grid.rows.row_data(row) {
            item.logo = image.clone();
            item.has_logo = true;
            state.grid.rows.set_row_data(row, item);
        }
    }
}

// Private API
impl Session {
    /// After the cursor or the row moved: keeps the cursor in the window,
    /// then redraws.
    fn follow_cursor(&self) {
        let visible = self.visible_minutes();
        let moved = {
            let mut state = self.state.borrow_mut();
            let grid = &mut state.grid;
            let end = grid.start + i64::from(visible) * 60 - HALF_HOUR;
            let start = if grid.cursor < grid.start {
                Some(half_hour(grid.cursor) - HALF_HOUR)
            } else if grid.cursor >= end {
                Some(half_hour(grid.cursor) - 2 * HALF_HOUR)
            } else {
                None
            };
            if let Some(start) = start {
                grid.start = start;
                grid.programmes.clear();
            }
            start.is_some()
        };
        if moved {
            self.refill_grid();
        } else {
            let row = self.state.borrow().grid.row;
            self.fill_grid(row..row + 1);
            self.redraw_grid();
        }
        if let Some(app) = self.app.upgrade() {
            app.set_guide_row(self.state.borrow().grid.row as i32);
            app.invoke_reveal_guide_row();
        }
    }

    /// Reloads every row on screen, after the window moved.
    fn refill_grid(&self) {
        let (visible, row) = {
            let state = self.state.borrow();
            (state.grid.visible.clone(), state.grid.row)
        };
        self.fill_grid(visible);
        self.fill_grid(row..row + 1);
        self.redraw_grid();
    }

    /// Loads the programmes of `rows` that are not loaded yet.
    fn fill_grid(&self, rows: Range<usize>) {
        let guide = self.guide.borrow();
        let Some(store) = guide.as_ref() else {
            return;
        };
        let mut state = self.state.borrow_mut();
        let start = state.grid.start;
        let count = state.grid.rows.row_count();
        for row in rows.start.min(count)..rows.end.min(count) {
            if state.grid.programmes.contains_key(&row) {
                continue;
            }
            let programmes = state
                .row_guides
                .get(row)
                .filter(|key| !key.channel.is_empty())
                .and_then(|key| {
                    store
                        .between(&key.provider, &key.channel, start, start + WINDOW)
                        .ok()
                })
                .unwrap_or_default();
            state.grid.programmes.insert(row, programmes);
        }
        drop(state);
        self.paint_rows(rows);
    }

    /// Rebuilds the cells of `rows` from the loaded programmes.
    fn paint_rows(&self, rows: Range<usize>) {
        let now = now();
        let state = self.state.borrow();
        let grid = &state.grid;
        for row in rows.start..rows.end.min(grid.rows.row_count()) {
            let (Some(programmes), Some(mut item)) =
                (grid.programmes.get(&row), grid.rows.row_data(row))
            else {
                continue;
            };
            let selected = (row == grid.row).then_some(grid.cursor);
            let cells = cells(
                programmes,
                grid.start,
                now,
                selected,
                state.row_archive.get(row).copied().unwrap_or(0),
            );
            item.cells = ModelRc::new(VecModel::from(cells));
            grid.rows.set_row_data(row, item);
        }
    }

    /// Repaints the selection, the ruler, and the details.
    fn redraw_grid(&self) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        let now = now();
        let visible = self.visible_minutes();
        let (start, cursor, visible_rows, row) = {
            let state = self.state.borrow();
            (
                state.grid.start,
                state.grid.cursor,
                state.grid.visible.clone(),
                state.grid.row,
            )
        };
        // Repaint the visible rows: the selection may have left one of them.
        self.paint_rows(visible_rows);
        self.paint_rows(row.saturating_sub(1)..row + 2);

        let ticks: Vec<GuideTick> = (0..=i64::from(visible) / 30 + 1)
            .map(|i| {
                let at = start + i * HALF_HOUR;
                GuideTick {
                    x: (i * 30) as f32,
                    label: clock(at).into(),
                }
            })
            .collect();
        app.set_guide_ticks(ModelRc::new(VecModel::from(ticks)));
        let now_x = (now - start) as f32 / 60.0;
        app.set_guide_now_x(if (0.0..visible as f32).contains(&now_x) {
            now_x
        } else {
            -1.0
        });
        app.set_guide_day(day_label(cursor, now).into());
        app.set_guide_details(self.grid_details(now));
    }

    fn grid_details(&self, now: i64) -> GuideDetails {
        let state = self.state.borrow();
        let grid = &state.grid;
        let channel = grid
            .rows
            .row_data(grid.row)
            .map(|r| r.name)
            .unwrap_or_default();
        let Some(programme) = grid
            .programmes
            .get(&grid.row)
            .and_then(|p| at(p, grid.cursor))
        else {
            return GuideDetails {
                title: channel.clone(),
                channel,
                when: "No guide for this time".into(),
                ..GuideDetails::default()
            };
        };
        let live = programme.start <= now && now < programme.stop;
        let archive_days = state.row_archive.get(grid.row).copied().unwrap_or(0);
        GuideDetails {
            title: programme.title.as_str().into(),
            channel,
            time: format!(
                "{} – {}",
                day_time(programme.start, now),
                clock(programme.stop)
            )
            .into(),
            when: when(programme, now).into(),
            description: programme.description.as_str().into(),
            live,
            replay: replayable(programme, archive_days, now),
        }
    }

    /// Minutes of programme the grid shows at its current width.
    fn visible_minutes(&self) -> i32 {
        self.app
            .upgrade()
            .map_or(180, |app| {
                (app.get_guide_track_width() / PX_PER_MINUTE) as i32
            })
            .max(60)
    }
}

/// Cells for `programmes` relative to `start`; the one airing at
/// `selected` is marked.
fn cells(
    programmes: &[Programme],
    start: i64,
    now: i64,
    selected: Option<i64>,
    archive_days: u32,
) -> Vec<GuideCell> {
    programmes
        .iter()
        .map(|p| {
            let shown = p.start.max(start);
            let state = if p.stop <= now {
                0
            } else if p.start <= now {
                1
            } else {
                2
            };
            GuideCell {
                title: p.title.as_str().into(),
                time: format!("{} – {}", clock(p.start), clock(p.stop)).into(),
                x: (shown - start) as f32 / 60.0,
                width: (p.stop - shown) as f32 / 60.0,
                state,
                selected: selected.is_some_and(|t| p.start <= t && t < p.stop),
                replay: replayable(p, archive_days, now),
            }
        })
        .collect()
}

/// The programme airing at `time`.
fn at(programmes: &[Programme], time: i64) -> Option<&Programme> {
    programmes.iter().find(|p| p.start <= time && time < p.stop)
}

/// The cursor after moving `dx` programmes from `cursor`; half an hour per
/// step where the row has no programme.
fn step(programmes: &[Programme], cursor: i64, dx: i32) -> i64 {
    let index = programmes
        .iter()
        .position(|p| p.start <= cursor && cursor < p.stop);
    let target = match index {
        Some(i) => i as i64 + i64::from(dx.signum()),
        None if dx > 0 => programmes
            .iter()
            .position(|p| p.start > cursor)
            .map_or(-1, |i| i as i64),
        None => programmes
            .iter()
            .rposition(|p| p.stop <= cursor)
            .map_or(-1, |i| i as i64),
    };
    usize::try_from(target)
        .ok()
        .and_then(|i| programmes.get(i))
        .map_or(cursor + i64::from(dx.signum()) * HALF_HOUR, |p| p.start)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(start: i64, stop: i64) -> Programme {
        Programme {
            start,
            stop,
            title: format!("{start}"),
            description: String::new(),
        }
    }

    #[test]
    fn step_moves_between_programmes_and_over_gaps() {
        let row = [p(0, 100), p(100, 200), p(300, 400)];
        assert_eq!(step(&row, 50, 1), 100);
        assert_eq!(step(&row, 150, -1), 0);
        assert_eq!(step(&row, 150, 1), 300, "over the gap");
        assert_eq!(step(&row, 250, -1), 100, "from inside the gap");
        assert_eq!(step(&row, 350, 1), 350 + HALF_HOUR, "past the end");
        assert_eq!(
            step(&[], 0, -1),
            -HALF_HOUR,
            "an empty row moves by half hours"
        );
    }

    #[test]
    fn cells_clip_to_the_window_and_mark_state() {
        let row = [p(-1800, 1800), p(1800, 3600), p(3600, 7200)];
        let cells = cells(&row, 0, 2000, Some(2000), 7);
        assert_eq!(
            (cells[0].x, cells[0].width),
            (0.0, 30.0),
            "starts before the window"
        );
        assert_eq!((cells[1].x, cells[1].width), (30.0, 30.0));
        assert_eq!(cells.iter().map(|c| c.state).collect::<Vec<_>>(), [0, 1, 2]);
        assert!(cells[1].selected && !cells[0].selected);
        assert!(
            cells[0].replay && !cells[1].replay,
            "only ended programmes replay"
        );
    }
}
