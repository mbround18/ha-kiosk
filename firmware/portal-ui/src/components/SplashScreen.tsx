import { Button } from '@/components/ui/button'

interface SplashScreenProps {
  onContinue: () => void
}

export function SplashScreen({ onContinue }: SplashScreenProps) {
  return (
    <div className="flex min-h-svh flex-col items-center justify-center gap-6 p-6 text-center">
      <div>
        <h1 className="text-3xl font-semibold tracking-tight">Welcome to HA Kiosk</h1>
        <p className="text-muted-foreground mt-2">Let's get your device connected.</p>
      </div>
      <Button size="lg" onClick={onContinue}>
        Get Started
      </Button>
    </div>
  )
}
