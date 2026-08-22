import { useEffect, useState } from 'react'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { api, type DashboardInfo } from '@/lib/api'

interface DashboardStepProps {
  onSelected: (dashboard: DashboardInfo) => void
}

export function DashboardStep({ onSelected }: DashboardStepProps) {
  const [dashboards, setDashboards] = useState<DashboardInfo[] | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [saving, setSaving] = useState<string | null>(null)

  useEffect(() => {
    api
      .dashboards()
      .then((res) => setDashboards(res.dashboards))
      .catch(() => setError('Could not load dashboards. Try reloading this page.'))
  }, [])

  const handleSelect = async (dashboard: DashboardInfo) => {
    setSaving(dashboard.url_path)
    try {
      await api.selectDashboard(dashboard)
      onSelected(dashboard)
    } finally {
      setSaving(null)
    }
  }

  return (
    <Card className="w-full max-w-md">
      <CardHeader>
        <CardTitle>Choose a dashboard</CardTitle>
        <CardDescription>Pick which Home Assistant dashboard this kiosk should show.</CardDescription>
      </CardHeader>
      <CardContent className="flex flex-col gap-3">
        {error && (
          <Alert variant="destructive">
            <AlertDescription>{error}</AlertDescription>
          </Alert>
        )}
        {dashboards === null && !error && (
          <p className="text-muted-foreground text-sm">Loading dashboards…</p>
        )}
        {dashboards?.length === 0 && (
          <Alert>
            <AlertDescription>
              No dashboards found yet. Create one in Home Assistant, then reload this page.
            </AlertDescription>
          </Alert>
        )}
        {dashboards?.map((d) => (
          <Button
            key={d.url_path}
            variant="outline"
            className="justify-start"
            disabled={saving !== null}
            onClick={() => handleSelect(d)}
          >
            {saving === d.url_path ? 'Saving…' : d.title}
          </Button>
        ))}
      </CardContent>
    </Card>
  )
}
