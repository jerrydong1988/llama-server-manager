/** @type {import('tailwindcss').Config} */
export default {
  content: [
    "./index.html",
    "./src/**/*.{js,ts,jsx,tsx}",
  ],
  darkMode: 'class',
  theme: {
    extend: {
      fontFamily: {
        sans: ['Geist', 'Microsoft YaHei UI', 'PingFang SC', 'sans-serif'],
        mono: ['JetBrains Mono', 'Microsoft YaHei UI', 'monospace'],
      },
      borderRadius: { lg: '12px', xl: '18px', '2xl': '24px' },
      colors: {
        slate: {
          50: '#f1f6f8', 100: '#e9f0f4', 200: '#dce6eb', 300: '#bbcabf',
          400: '#8eaaa7', 500: '#677e8c', 600: '#506679', 700: '#344b5b',
          800: '#203442', 900: '#0d1c2d', 950: '#010f1f',
        },
        blue: {
          50: '#e0f2fa', 100: '#c8e8f5', 200: '#a1dcf2', 300: '#7bd0ff',
          400: '#43b5df', 500: '#189acb', 600: '#00779e', 700: '#08647f',
          800: '#0c4d64', 900: '#073346', 950: '#062536',
        },
      },
    },
  },
  plugins: [],
}
