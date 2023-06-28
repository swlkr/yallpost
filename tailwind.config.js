/** @type {import('tailwindcss').Config} */
module.exports = {
  content: ["./src/**/*.rs"],
  theme: {
    extend: {
      screens: {
        standalone: { raw: "(display-mode: standalone)" }
      }
    },
  },
  plugins: [],
}
