// Text for the preview, laid out exactly like the export compositor does it
// (`text.rs`): glyphs are placed by their advance widths alone and vertical
// positions never depend on font metrics.

import interBold from "../../assets/fonts/Inter-Bold.otf?url";
import interRegular from "../../assets/fonts/Inter-Regular.otf?url";
import plexBold from "../../assets/fonts/IBMPlexMono-Bold.ttf?url";
import plexRegular from "../../assets/fonts/IBMPlexMono-Regular.ttf?url";
import poppinsBold from "../../assets/fonts/Poppins-Bold.ttf?url";
import poppinsRegular from "../../assets/fonts/Poppins-Regular.ttf?url";
import serifBold from "../../assets/fonts/PTSerif-Bold.ttf?url";
import serifRegular from "../../assets/fonts/PTSerif-Regular.ttf?url";

/** Font families a project can use. The first is the fallback. */
export const FAMILIES = ["Inter", "Poppins", "PT Serif", "IBM Plex Mono"] as const;

export const LINE_HEIGHT = 1.25;
export const FIRST_BASELINE = 0.95;
export const CENTER_BASELINE = 0.35;

const FILES: Record<string, [string, string]> = {
  Inter: [interRegular, interBold],
  Poppins: [poppinsRegular, poppinsBold],
  "PT Serif": [serifRegular, serifBold],
  "IBM Plex Mono": [plexRegular, plexBold],
};

// Private names, so a system font called "Inter" can never stand in for the
// bundled one and shift the layout.
const cssName = (family: string) => `Cinesnap ${family}`;

function known(family: string | undefined): string {
  return FAMILIES.find((f) => f.toLowerCase() === (family ?? "").toLowerCase()) ?? FAMILIES[0];
}

let loading: Promise<void> | null = null;

/** Load the bundled fonts. Resolves when text can be measured and drawn. */
export function loadFonts(): Promise<void> {
  if (loading) return loading;
  if (typeof FontFace === "undefined" || typeof document === "undefined") return (loading = Promise.resolve());
  const faces = Object.entries(FILES).flatMap(([family, [regular, bold]]) => [new FontFace(cssName(family), `url(${regular})`, { weight: "400" }), new FontFace(cssName(family), `url(${bold})`, { weight: "700" })]);
  loading = Promise.all(
    faces.map((f) =>
      f.load().then(
        (loaded) => void document.fonts.add(loaded),
        () => undefined,
      ),
    ),
  ).then(() => {
    advances.clear();
  });
  return loading;
}

export interface TextStyle {
  family?: string;
  bold: boolean;
  px: number;
}

function css(style: TextStyle, px: number): string {
  return `${style.bold ? 700 : 400} ${px}px "${cssName(known(style.family))}", sans-serif`;
}

// Advance of each character at 1000 px, per face. Advances scale linearly
// with the font size, so one measurement serves every size.
const REF = 1000;
const advances = new Map<string, Map<string, number>>();

type Ctx = CanvasRenderingContext2D;

function advance(ctx: Ctx, style: TextStyle, ch: string): number {
  const face = `${known(style.family)}|${style.bold}`;
  let table = advances.get(face);
  if (!table) advances.set(face, (table = new Map()));
  let w = table.get(ch);
  if (w === undefined) {
    const prev = ctx.font;
    ctx.font = css(style, REF);
    w = ctx.measureText(ch).width;
    ctx.font = prev;
    table.set(ch, w);
  }
  return (w * style.px) / REF;
}

/** Width of a single line of text. */
export function measure(ctx: Ctx, style: TextStyle, text: string): number {
  let w = 0;
  for (const ch of text) w += advance(ctx, style, ch);
  return w;
}

/** Draw one line of text with its baseline starting at (x, baseline). */
export function drawText(ctx: Ctx, style: TextStyle, text: string, x: number, baseline: number, color: string, alpha = 1): void {
  if (alpha <= 0 || style.px < 1) return;
  ctx.save();
  ctx.globalAlpha *= Math.min(1, alpha);
  ctx.font = css(style, style.px);
  ctx.fillStyle = color;
  ctx.textBaseline = "alphabetic";
  ctx.textAlign = "left";
  let pen = x;
  for (const ch of text) {
    ctx.fillText(ch, pen, baseline);
    pen += advance(ctx, style, ch);
  }
  ctx.restore();
}
