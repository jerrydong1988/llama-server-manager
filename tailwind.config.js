/** @type {import('tailwindcss').Config} */
export default {
  content: [
    "./index.html",
    "./src/**/*.{js,ts,jsx,tsx}",
  ],
  darkMode: 'class',
  theme: {
    extend: {
      colors: {
        ui: Object.fromEntries([
          'canvas', 'surface', 'inset', 'hover', 'line', 'control', 'ink', 'body', 'muted',
          'accent', 'action', 'action-ink', 'action-line', 'selected', 'positive', 'warn',
          'warn-bg', 'danger', 'danger-bg', 'info', 'info-bg', 'focus', 'scrim',
        ].map(role => [role, `var(--ui-${role})`])),
      },
    },
  },
  plugins: [],
}
