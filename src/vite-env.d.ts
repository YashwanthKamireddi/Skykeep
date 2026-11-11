/// <reference types="vite/client" />

/** Every open-source licence the app ships, written by the build (scripts/licences.mjs). */
declare module "virtual:licences" {
  /** One licence text and everything shipped under it, each as "name version". */
  export interface Licence {
    id: string;
    name: string;
    text: string;
    users: string[];
  }
  const licences: Licence[];
  export default licences;
}
