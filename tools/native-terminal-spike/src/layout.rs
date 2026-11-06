use crate::Snapshot;

pub const CELL_WIDTH: f32 = 11.0;
pub const CELL_HEIGHT: f32 = 23.0;
pub const PADDING: f32 = 12.0;

#[derive(Debug)]
pub struct Placement {
    pub index: usize,
    pub left: f32,
    pub top: f32,
    pub width: f32,
}

pub fn placements(snapshot: &Snapshot) -> Vec<Placement> {
    snapshot
        .cells
        .iter()
        .enumerate()
        .filter(|(_, cell)| cell.width > 0 && cell.text != " ")
        .map(|(index, cell)| Placement {
            index,
            left: PADDING + (index % snapshot.cols) as f32 * CELL_WIDTH,
            top: PADDING + (index / snapshot.cols) as f32 * CELL_HEIGHT,
            width: cell.width as f32 * CELL_WIDTH,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Terminal;

    #[test]
    fn unicode_wide_combining_and_proportional_fallback_cannot_shift_columns() {
        let mut terminal = Terminal::new(20, 4);
        terminal.feed("Wi中e\u{301}🙂Z\r\nאבX".as_bytes());
        let grid = terminal.snapshot();
        let placed = placements(&grid);
        let positions: Vec<_> = placed
            .iter()
            .filter(|p| p.top == PADDING)
            .map(|p| (p.index, p.left, p.width))
            .collect();
        assert_eq!(
            positions,
            [
                (0, 12.0, 11.0),
                (1, 23.0, 11.0),
                (2, 34.0, 22.0),
                (4, 56.0, 11.0),
                (5, 67.0, 22.0),
                (7, 89.0, 11.0)
            ]
        );
        let second: Vec<_> = placed
            .iter()
            .filter(|p| p.top == PADDING + CELL_HEIGHT)
            .map(|p| p.left)
            .collect();
        assert_eq!(second, [12.0, 23.0, 34.0]);
        assert_eq!(grid.cells[4].text, "e\u{301}");
        assert!(placed.iter().all(|p| grid.cells[p.index].width > 0));
    }
}
