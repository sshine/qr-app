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

#let rule() = line(length: 100%, stroke: (paint: luma(60%), thickness: 0.3pt))

// The address, turned on its side in the gutter beside the symbol.
//
// Rotated because it is the longest run of characters on the sticker and the one with
// the least claim on width: upright it sets the caption's type size for every label,
// and on its side it costs a column as narrow as the type is tall. reflow makes the
// layout account for the turned box rather than the original one.
#let host() = rotate(-90deg, reflow: true, text(
  size: data.host_pt * 1pt,
  fill: luma(45%),
)[#data.host])

// What is printed beside the symbol: the code, a line, and the secret under it.
//
// The line is there to be read across, not written on: it separates the part that
// identifies the sticker from the part that opens it.
#let caption(c) = {
  text(size: data.caption_pt * 1pt, weight: "bold")[#c.label]
  if L.print_secret {
    v(data.rule_gap_mm * 1mm)
    rule()
    v(data.rule_gap_mm * 1mm)
    for (i, line) in c.secret_lines.enumerate() {
      if i > 0 { linebreak() }
      text(size: data.secret_pt * 1pt, fill: luma(35%))[#line]
    }
  }
  for _ in range(L.label_lines) {
    v(data.rule_gap_mm * 1mm)
    rule()
  }
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
  grid(
    columns: (data.qr_box_mm * 1mm, data.host_w_mm * 1mm, 1fr),
    column-gutter: 1.5mm,
    align: (center + horizon, center + horizon, left + horizon),
    image(c.svg, width: data.qr_box_mm * 1mm), host(), caption(c),
  ),
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
