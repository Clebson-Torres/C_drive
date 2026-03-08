/** @type {import('tailwindcss').Config} */
export default {
  content: ["./index.html", "./src/ui-app/**/*.{js,jsx}"],
  theme: {
    extend: {
      colors: {
        ink: "#08111f",
        panel: "#edf4ff",
        brand: "#198cff",
        brandDark: "#0e5dc7",
      },
      boxShadow: {
        glass: "0 24px 60px rgba(12, 25, 53, 0.16)",
      },
      backgroundImage: {
        "shell-gradient":
          "radial-gradient(circle at top left, rgba(62, 149, 255, 0.28), transparent 32%), radial-gradient(circle at top right, rgba(0, 197, 255, 0.16), transparent 22%), linear-gradient(180deg, #d9e8ff 0%, #f5f8ff 100%)",
      },
    },
  },
  plugins: [],
};
