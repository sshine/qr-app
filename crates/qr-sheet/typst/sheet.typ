// A sheet of QR stickers.
//
// Every position comes from codes.json in millimetres and is placed absolutely, because
// die-cut label stock has no cut guides to aim at and a grid with gutters cannot express
// where a manufacturer put the first label.
//
// Only fonts typst embeds are named here. The renderer passes --ignore-system-fonts, so
// a sheet looks the same from a laptop with a full font tree and from a container with
// none.

#let data = json("codes.json")
#let L = data.layout

#set page(paper: "a4", margin: 0pt)
#set text(font: "DejaVu Sans Mono", size: 7pt, fill: black)

#let hair = 0.3pt + luma(45%)

// Where to type, then what to type there, one run to a line.
//
// Set tight: the row it sits in is measured in millimetres on the Rust side, so the
// leading has to be the one that measurement assumed.
#let address(c) = {
  set par(leading: 0.3em)
  set text(size: data.url_pt * 1pt)
  for (i, line) in c.url_lines.enumerate() {
    if i > 0 { linebreak() }
    line
  }
}

// Everything beside the symbol, in a box drawn the symbol's own height.
//
// A table rather than a stack of lines: the secret is typed a character at a time, and a
// cell is somewhere for the eye to come back to after each look at the keyboard. Row
// heights are explicit because a table has no height of its own to divide, and the whole
// point of this box is that it ends where the symbol ends.
#let meta(c) = {
  let rows = (data.header_h_mm * 1mm,)
  if data.row_h_mm > 0 {
    let body = c.cells.len() / data.cols + data.blank_rows
    rows += (data.row_h_mm * 1mm,) * int(body)
  }

  table(
    columns: (1fr,) * data.cols,
    rows: rows,
    stroke: hair,
    inset: 0pt,
    align: center + horizon,
    table.cell(
      colspan: data.cols,
      inset: (x: data.box_pad_mm * 1mm, y: data.box_pad_mm * 1mm),
      align: left + horizon,
      address(c),
    ),
    ..c.cells.map(ch => text(size: data.cell_pt * 1pt)[#ch]),
    // Blank, full width, and ruled off from each other: somewhere to write what this
    // sticker went on.
    ..range(data.blank_rows).map(_ => table.cell(colspan: data.cols)[]),
  )
}

#let sticker(c) = box(
  width: L.label_w_mm * 1mm,
  height: L.label_h_mm * 1mm,
  inset: (x: L.inset_x_mm * 1mm, y: L.inset_y_mm * 1mm),
  stroke: if L.cut_guides {
    (paint: luma(70%), thickness: 0.3pt, dash: "dashed")
  } else {
    none
  },
  // Centred down the label by hand: the image is the bare symbol, so nothing above or
  // below it pushes it off the label's top edge.
  align(horizon, grid(
    columns: (data.qr_box_mm * 1mm, data.box_w_mm * 1mm),
    column-gutter: 1.5mm,
    align: (center + horizon, center + horizon),
    image(c.svg, width: L.qr_mm * 1mm),
    box(width: data.box_w_mm * 1mm, meta(c)),
  )),
)

#let per-page = L.cols * L.rows
#let total = data.codes.len()
#let pages = calc.max(1, calc.ceil(total / per-page))

#for p in range(pages) {
  if p > 0 { pagebreak() }

  let lo = p * per-page
  let hi = calc.min(lo + per-page, total)

  for i in range(lo, hi) {
    let k = i - lo
    place(
      dx: (L.margin_left_mm + calc.rem(k, L.cols) * L.pitch_x_mm) * 1mm,
      dy: (L.margin_top_mm + calc.quo(k, L.cols) * L.pitch_y_mm) * 1mm,
      sticker(data.codes.at(i)),
    )
  }

  // Omitted outright when the last row reaches the foot of the page, rather than
  // overprinted onto it.
  if data.footer_dy_mm != none {
    place(
      dx: 6mm,
      dy: data.footer_dy_mm * 1mm,
      text(size: 6pt, fill: luma(55%), font: "Libertinus Serif")[#data.footer],
    )
  }
}
