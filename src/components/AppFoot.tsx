/**
 * The foot of an app's own sidebar, in an app's own window (Library's `SOLO`): the account and
 * how current the app is, as the main window's sidebar shows them. Every app's sidebar ends
 * with one; in the main window, where its sidebar already has them, it draws nothing.
 */
import { createContext, useContext, type ReactNode } from "react";

export const AppFootContext = createContext<ReactNode>(null);

export function AppFoot() {
  const foot = useContext(AppFootContext);
  return foot ? <div className="app-foot">{foot}</div> : null;
}
