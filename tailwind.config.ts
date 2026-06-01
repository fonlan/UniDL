import type { Config } from "tailwindcss";

export default {
  darkMode: "class",
  content: ["./index.html", "./src/**/*.{ts,tsx}", "./shared/**/*.ts"],
  theme: {
    extend: {
      fontFamily: {
        sans: [
          "Inter",
          "Segoe UI",
          "Microsoft YaHei UI",
          "Microsoft YaHei",
          "system-ui",
          "sans-serif",
        ],
      },
      colors: {
        surface: "#f7f8fa",
        ink: "#171a1f",
      },
    },
  },
  plugins: [],
} satisfies Config;
