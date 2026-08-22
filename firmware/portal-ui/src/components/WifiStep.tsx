import { useEffect, useState } from 'react'
import { Card, CardContent, CardDescription, CardFooter, CardHeader, CardTitle } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Alert, AlertDescription } from '@/components/ui/alert'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'
import { api, type NetworkInfo, type WifiJoinFailureReason } from '@/lib/api'

const FAILURE_MESSAGES: Record<WifiJoinFailureReason, string> = {
  wrong_password: "That password didn't work. Double-check it and try again.",
  network_not_found: "Couldn't find that network. It may be out of range.",
  other: 'Something went wrong connecting. Please try again.',
}

interface WifiStepProps {
  onConnected: () => void
}

export function WifiStep({ onConnected }: WifiStepProps) {
  const [networks, setNetworks] = useState<NetworkInfo[]>([])
  const [ssid, setSsid] = useState('')
  const [password, setPassword] = useState('')
  const [connecting, setConnecting] = useState(false)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    api
      .networks()
      .then((res) => {
        setNetworks(res.networks)
        if (res.networks.length > 0) setSsid(res.networks[0].ssid)
      })
      .catch(() => setError('Could not scan for networks. Try reloading this page.'))
  }, [])

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault()
    setConnecting(true)
    setError(null)
    try {
      const result = await api.joinWifi(ssid, password)
      if (result.status === 'connected') {
        onConnected()
      } else {
        setError(FAILURE_MESSAGES[result.reason])
      }
    } catch {
      setError(FAILURE_MESSAGES.other)
    } finally {
      setConnecting(false)
    }
  }

  return (
    <Card className="w-full max-w-md">
      <CardHeader>
        <CardTitle>Connect to your home network</CardTitle>
        <CardDescription>Choose your WiFi network and enter its password.</CardDescription>
      </CardHeader>
      <form onSubmit={handleSubmit}>
        <CardContent className="flex flex-col gap-4">
          <div className="flex flex-col gap-2">
            <Label htmlFor="ssid">Network</Label>
            {networks.length > 0 ? (
              <Select value={ssid} onValueChange={(value) => setSsid(value ?? '')}>
                <SelectTrigger id="ssid" className="w-full">
                  <SelectValue placeholder="Select a network" />
                </SelectTrigger>
                <SelectContent>
                  {networks.map((n) => (
                    <SelectItem key={n.ssid} value={n.ssid}>
                      {n.ssid} {n.secure ? '🔒' : ''} ({n.rssi} dBm)
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            ) : (
              <Input
                id="ssid"
                value={ssid}
                onChange={(e) => setSsid(e.target.value)}
                placeholder="Network name"
              />
            )}
          </div>
          <div className="flex flex-col gap-2">
            <Label htmlFor="password">Password</Label>
            <Input
              id="password"
              type="password"
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              autoComplete="off"
            />
          </div>
          {error && (
            <Alert variant="destructive">
              <AlertDescription>{error}</AlertDescription>
            </Alert>
          )}
        </CardContent>
        <CardFooter>
          <Button type="submit" disabled={connecting || !ssid} className="w-full">
            {connecting ? 'Connecting…' : 'Connect'}
          </Button>
        </CardFooter>
      </form>
    </Card>
  )
}
