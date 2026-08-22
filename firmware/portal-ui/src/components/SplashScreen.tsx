import { QRCodeSVG } from 'qrcode.react'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import { Separator } from '@/components/ui/separator'
import type { PortalInfo } from '@/lib/api'

interface SplashScreenProps {
  portalInfo: PortalInfo | null
  onContinue: () => void
}

export function SplashScreen({ portalInfo, onContinue }: SplashScreenProps) {
  const portalUrl = portalInfo ? `http://${portalInfo.ip}/` : null
  // Open network (no password) - contracts/captive-portal-api.md, AuthMethod::None on the AP.
  const wifiQrValue = portalInfo ? `WIFI:T:nopass;S:${portalInfo.ssid};;` : null

  return (
    <div className="flex min-h-svh flex-col items-center justify-center gap-8 p-6">
      <div className="text-center">
        <h1 className="text-3xl font-semibold tracking-tight">Welcome to HA Kiosk</h1>
        <p className="text-muted-foreground mt-2">Let's get your device connected.</p>
      </div>

      <div className="grid w-full max-w-2xl gap-6 sm:grid-cols-2">
        <Card>
          <CardHeader>
            <CardTitle>1. Connect to WiFi</CardTitle>
            <CardDescription>
              Scan with your phone's camera to join{' '}
              <span className="font-medium text-foreground">{portalInfo?.ssid ?? '...'}</span>
            </CardDescription>
          </CardHeader>
          <CardContent className="flex justify-center">
            {wifiQrValue ? (
              <QRCodeSVG value={wifiQrValue} size={192} />
            ) : (
              <div className="size-48 animate-pulse rounded-md bg-muted" />
            )}
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle>2. Open Setup</CardTitle>
            <CardDescription>
              Already connected? Scan to open this page directly.
            </CardDescription>
          </CardHeader>
          <CardContent className="flex justify-center">
            {portalUrl ? (
              <QRCodeSVG value={portalUrl} size={192} />
            ) : (
              <div className="size-48 animate-pulse rounded-md bg-muted" />
            )}
          </CardContent>
        </Card>
      </div>

      <Separator className="max-w-2xl" />

      <div className="flex flex-col items-center gap-3 text-center">
        <p className="text-muted-foreground text-sm">
          You're already viewing this page, so you're connected. Continue when ready.
        </p>
        <Button size="lg" onClick={onContinue}>
          Get Started
        </Button>
      </div>
    </div>
  )
}
