/** WCAG relative luminance of an sRGB color (0–255 channels). */
export function relativeLuminance(r: number, g: number, b: number): number {
  const lin = (c: number) => {
    const s = c / 255;
    return s <= 0.04045 ? s / 12.92 : Math.pow((s + 0.055) / 1.055, 2.4);
  };
  return 0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b);
}

/** WCAG contrast ratio between two luminances. */
export function contrastRatio(L1: number, L2: number): number {
  const a = Math.max(L1, L2) + 0.05;
  const b = Math.min(L1, L2) + 0.05;
  return a / b;
}

export interface AccentPaint {
  /** Possibly clamped accent, as `r, g, b`. */
  rgb: string;
  css: string;
  dim: string;
  /** Foreground (icon/text) that contrasts >= 4.5:1 with the accent. */
  fg: string;
  fgRgb: string;
}

/** Pick a readable foreground and, if needed, a slightly darker/lighter
 *  accent so the contrast ratio is at least 4.5:1 (WCAG AA for text). */
export function paintAccent(r: number, g: number, b: number): AccentPaint {
  let rgb = [r, g, b].map((c) => Math.max(0, Math.min(255, Math.round(c))));
  const whiteL = 1;
  const blackL = 0;
  let L = relativeLuminance(rgb[0], rgb[1], rgb[2]);

  // Near-white or near-black accents make terrible buttons — nudge them.
  if (L > 0.85) {
    rgb = shadeToward(rgb, [20, 19, 18], 0.28);
    L = relativeLuminance(rgb[0], rgb[1], rgb[2]);
  } else if (L < 0.08) {
    rgb = shadeToward(rgb, [245, 243, 240], 0.28);
    L = relativeLuminance(rgb[0], rgb[1], rgb[2]);
  }

  let fg: number[] = contrastRatio(L, blackL) >= contrastRatio(L, whiteL)
    ? [20, 19, 18]
    : [245, 243, 240];
  let fgL = relativeLuminance(fg[0], fg[1], fg[2]);

  // If still short of 4.5:1, push the accent away from the chosen fg.
  let guard = 0;
  while (contrastRatio(L, fgL) < 4.5 && guard++ < 16) {
    const toward = fgL > L ? [20, 19, 18] : [245, 243, 240];
    rgb = shadeToward(rgb, toward, 0.12);
    L = relativeLuminance(rgb[0], rgb[1], rgb[2]);
  }

  const [rr, gg, bb] = rgb;
  const [fr, fg_, fb] = fg;
  return {
    rgb: `${rr}, ${gg}, ${bb}`,
    css: `rgb(${rr}, ${gg}, ${bb})`,
    dim: `rgba(${rr}, ${gg}, ${bb}, 0.18)`,
    fg: `rgb(${fr}, ${fg_}, ${fb})`,
    fgRgb: `${fr}, ${fg_}, ${fb}`,
  };
}

function shadeToward(rgb: number[], toward: number[], t: number): number[] {
  return rgb.map((c, i) => Math.round(c + (toward[i] - c) * t));
}
