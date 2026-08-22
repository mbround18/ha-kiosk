import { useState } from 'react'
import { Card, CardContent, CardDescription, CardFooter, CardHeader, CardTitle } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { api, type HaConnectFailureReason } from '@/lib/api'

const FAILURE_MESSAGES: Record<HaConnectFailureReason, string> = {
  unreachable: "Couldn't reach that address. Double-check the URL and try again.",
  unauthorized: 'That address was reached, but the token was rejected.',
}

interface HaStepProps {
  onConnected: (instanceName: string) => void
}

export function HaStep({ onConnected }: HaStepProps) {
  const [baseUrl, setBaseUrl] = useState('http://homeassistant.local:8123')
  const [token, setToken] = useState('')
  const [connecting, setConnecting] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault()
    setConnecting(true)
    setError(null)
    try {
      const result = await api.connectHomeAssistant(baseUrl, token)
      if (result.status === 'connected') {
        onConnected(result.instance_name)
      } else {
        setError(FAILURE_MESSAGES[result.reason])
      }
    } catch {
      setError(FAILURE_MESSAGES.unreachable)
    } finally {
      setConnecting(false)
    }
  }

  return (
    <Card className="w-full max-w-md">
      <CardHeader>
        <CardTitle>Connect to Home Assistant</CardTitle>
        <CardDescription>
          Enter your Home Assistant address and a long-lived access token. Generate one from your
          profile page, under "Security" → "Long-lived access tokens".
        </CardDescription>
      </CardHeader>
      <form onSubmit={handleSubmit}>
        <CardContent className="flex flex-col gap-4">
          <div className="flex flex-col gap-2">
            <Label htmlFor="base_url">Home Assistant URL</Label>
            <Input
              id="base_url"
              value={baseUrl}
              onChange={(e) => setBaseUrl(e.target.value)}
              placeholder="http://homeassistant.local:8123"
            />
          </div>
          <div className="flex flex-col gap-2">
            <Label htmlFor="token">Access token</Label>
            <Input
              id="token"
              type="password"
              value={token}
              onChange={(e) => setToken(e.target.value)}
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
          <Button type="submit" disabled={connecting || !baseUrl || !token} className="w-full">
            {connecting ? 'Connecting…' : 'Connect'}
          </Button>
        </CardFooter>
      </form>
    </Card>
  )
}
