// Typed client for the device's captive-portal HTTP API.
// Mirrors specs/001-wifi-ha-provisioning/contracts/captive-portal-api.md.

export interface PortalInfo {
  ssid: string
  ip: string
}

export interface StatusResponse {
  wifi_configured: boolean
  ha_configured: boolean
  dashboard_selected: boolean
  instance_name?: string
}

export interface NetworkInfo {
  ssid: string
  rssi: number
  secure: boolean
}

export type WifiJoinFailureReason = 'wrong_password' | 'network_not_found' | 'other'

export type WifiJoinResult =
  | { status: 'connected' }
  | { status: 'failed'; reason: WifiJoinFailureReason }

export type HaConnectFailureReason = 'unreachable' | 'unauthorized'

export type HaConnectResult =
  | { status: 'connected'; instance_name: string }
  | { status: 'failed'; reason: HaConnectFailureReason }

export interface DashboardInfo {
  url_path: string
  title: string
}

async function getJson<T>(path: string): Promise<T> {
  const res = await fetch(path)
  return (await res.json()) as T
}

async function postJson<T>(path: string, body: unknown): Promise<T> {
  const res = await fetch(path, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(body),
  })
  return (await res.json()) as T
}

export const api = {
  portalInfo: () => getJson<PortalInfo>('/api/portal-info'),
  status: () => getJson<StatusResponse>('/api/status'),
  networks: () => getJson<{ networks: NetworkInfo[] }>('/api/networks'),
  joinWifi: (ssid: string, password: string) =>
    postJson<WifiJoinResult>('/api/wifi', { ssid, password }),
  connectHomeAssistant: (base_url: string, access_token: string) =>
    postJson<HaConnectResult>('/api/home-assistant', { base_url, access_token }),
  dashboards: () => getJson<{ dashboards: DashboardInfo[] }>('/api/dashboards'),
  selectDashboard: (dashboard: DashboardInfo) =>
    postJson<{ status: string }>('/api/dashboard', dashboard),
}
