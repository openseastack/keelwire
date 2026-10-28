/** @type {import('tailwindcss').Config} */
const v = (n) => `rgb(var(--${n}) / <alpha-value>)`;
export default {
  content: ["./index.html", "./src/**/*.{ts,tsx}"],
  theme: {
    extend: {
      colors: {
        paper: v("paper"),
        panel: v("panel"),
        shoal: v("shoal"),
        ink: v("ink"),
        muted: v("muted"),
        rule: v("rule"),
        port: v("port"),
        stbd: v("stbd"),
        caution: v("caution"),
        sea: v("sea"),
        c1: v("c1"),
        c2: v("c2"),
        c3: v("c3"),
        c4: v("c4"),
      },
      fontFamily: {
        sans: ['"Geist Sans"', "system-ui", "sans-serif"],
        cond: ['"Geist Sans"', "system-ui", "sans-serif"],
        mono: ['"Geist Mono"', "ui-monospace", "monospace"],
      },
    },
  },
  plugins: [],
};
