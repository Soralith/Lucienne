/** Tailwind config for the precompiled dashboard stylesheet.
 *
 * The class names are scanned from the minijinja templates and the dashboard
 * JavaScript, so any utility used there is emitted into
 * ui/static/dashboard/css/tailwind.css. Rebuild it after editing markup with
 * `npm run build:css`; `npm run check:css` fails if the committed file is stale.
 */
module.exports = {
  content: ["./ui/templates/**/*.html", "./ui/static/dashboard/js/**/*.js"],
  theme: {
    extend: {},
  },
  plugins: [],
};
