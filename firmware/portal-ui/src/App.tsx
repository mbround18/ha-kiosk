import { useEffect, useState } from 'react'
import { SplashScreen } from '@/components/SplashScreen'
import { WifiStep } from '@/components/WifiStep'
import { HaStep } from '@/components/HaStep'
import { DashboardStep } from '@/components/DashboardStep'
import { Badge } from '@/components/ui/badge'
import { api, type PortalInfo, type StatusResponse } from '@/lib/api'

type Screen = 'loading' | 'splash' | 'wifi' | 'ha' | 'dashboard' | 'done'

function nextIncompleteScreen(status: StatusResponse): Screen {
  if (!status.wifi_configured) return 'wifi'
  if (!status.ha_configured) return 'ha'
  if (!status.dashboard_selected) return 'dashboard'
  return 'done'
}

const STEPS: { key: Screen; label: string }[] = [
  { key: 'wifi', label: 'WiFi' },
  { key: 'ha', label: 'Home Assistant' },
  { key: 'dashboard', label: 'Dashboard' },
]

function StepIndicator({ current }: { current: Screen }) {
  const currentIndex = STEPS.findIndex((s) => s.key === current)
  return (
    <div className="flex items-center gap-2">
      {STEPS.map((step, i) => (
        <Badge key={step.key} variant={i === currentIndex ? 'default' : 'secondary'}>
          {i + 1}. {step.label}
        </Badge>
      ))}
    </div>
  )
}

export default function App() {
  const [screen, setScreen] = useState<Screen>('loading')
  const [portalInfo, setPortalInfo] = useState<PortalInfo | null>(null)
  const [instanceName, setInstanceName] = useState<string | null>(null)

  useEffect(() => {
    api
      .portalInfo()
      .then(setPortalInfo)
      .catch(() => {
        /* QR codes just won't render; the rest of the flow still works */
      })

    api
      .status()
      .then((status) => {
        if (status.instance_name) setInstanceName(status.instance_name)
        setScreen(status.wifi_configured || status.ha_configured ? nextIncompleteScreen(status) : 'splash')
      })
      .catch(() => setScreen('splash'))
  }, [])

  if (screen === 'loading') {
    return (
      <div className="flex min-h-svh items-center justify-center">
        <p className="text-muted-foreground">Loading…</p>
      </div>
    )
  }

  if (screen === 'splash') {
    return <SplashScreen portalInfo={portalInfo} onContinue={() => setScreen('wifi')} />
  }

  if (screen === 'done') {
    return (
      <div className="flex min-h-svh flex-col items-center justify-center gap-3 p-6 text-center">
        <h1 className="text-2xl font-semibold">All set!</h1>
        <p className="text-muted-foreground">
          {instanceName ? `Connected to ${instanceName}. ` : ''}
          This kiosk is ready to go.
        </p>
      </div>
    )
  }

  return (
    <div className="flex min-h-svh flex-col items-center justify-center gap-6 p-6">
      <StepIndicator current={screen} />
      {screen === 'wifi' && (
        <WifiStep onConnected={() => setScreen('ha')} />
      )}
      {screen === 'ha' && (
        <HaStep
          onConnected={(name) => {
            setInstanceName(name)
            setScreen('dashboard')
          }}
        />
      )}
      {screen === 'dashboard' && <DashboardStep onSelected={() => setScreen('done')} />}
    </div>
  )
}
