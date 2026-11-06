// Public buffer/cell/mode APIs, except cursor visibility, which headless 5.5.0
// only exposes through this version-pinned internal coreService field.
export function snapshot(term) {
  const buffer = term.buffer.active;
  const lines = [];
  const cells = [];
  for (let row = 0; row < buffer.length; row++) {
    const line = buffer.getLine(row);
    // Match the native snapshot's existing trailing-space text projection.
    // xterm trimRight only trims NULL cells, not explicit spaces from Vim EL.
    // Full cell characters/width/colors/flags below remain compared EXACTLY.
    lines.push(line.translateToString(false).replace(/ +$/u, ''));
    if (row < buffer.baseY || row >= buffer.baseY + term.rows) continue;
    const rowCells = [];
    for (let col = 0; col < term.cols; col++) {
      const cell = line.getCell(col);
      const width = cell.getWidth();
      const chars = width === 0 ? '' : cell.getChars() || ' ';
      const color = fg => {
        if (fg ? cell.isFgDefault() : cell.isBgDefault()) return ['default', 0];
        return [fg ? cell.isFgRGB() ? 'rgb' : 'palette' : cell.isBgRGB() ? 'rgb' : 'palette', fg ? cell.getFgColor() : cell.getBgColor()];
      };
      rowCells.push([chars, width, color(true), color(false), [cell.isBold(), cell.isItalic(), cell.isUnderline(), cell.isInverse(), cell.isInvisible(), cell.isStrikethrough(), cell.isDim()].map(Boolean)]);
    }
    cells.push(rowCells);
  }
  const modeNames = ['applicationCursorKeysMode', 'applicationKeypadMode', 'bracketedPasteMode', 'insertMode', 'originMode', 'sendFocusMode', 'wraparoundMode'];
  return { type: buffer.type, cursor: [buffer.cursorX, buffer.cursorY], history: buffer.baseY, lines, cells,
    modes: Object.fromEntries(modeNames.map(name => [name, term.modes[name]])), cursorVisible: !term._core.coreService.isCursorHidden };
}
