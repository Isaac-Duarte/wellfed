// Vite's `?url` import, typed here so the package needs no Vite of its own:
// the apps that build it bring Vite.
declare module "*.svg?url" {
  const url: string;
  export default url;
}
