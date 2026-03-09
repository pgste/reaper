{{/*
Expand the name of the chart.
*/}}
{{- define "reaper.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Create a default fully qualified app name.
*/}}
{{- define "reaper.fullname" -}}
{{- if .Values.fullnameOverride }}
{{- .Values.fullnameOverride | trunc 63 | trimSuffix "-" }}
{{- else }}
{{- $name := default .Chart.Name .Values.nameOverride }}
{{- if contains $name .Release.Name }}
{{- .Release.Name | trunc 63 | trimSuffix "-" }}
{{- else }}
{{- printf "%s-%s" .Release.Name $name | trunc 63 | trimSuffix "-" }}
{{- end }}
{{- end }}
{{- end }}

{{/*
Create chart name and version as used by the chart label.
*/}}
{{- define "reaper.chart" -}}
{{- printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Common labels
*/}}
{{- define "reaper.labels" -}}
helm.sh/chart: {{ include "reaper.chart" . }}
{{ include "reaper.selectorLabels" . }}
{{- if .Chart.AppVersion }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
{{- end }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
{{- end }}

{{/*
Selector labels
*/}}
{{- define "reaper.selectorLabels" -}}
app.kubernetes.io/name: {{ include "reaper.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}

{{/*
Create the name of the service account to use
*/}}
{{- define "reaper.serviceAccountName" -}}
{{- if .Values.serviceAccount.create }}
{{- default (include "reaper.fullname" .) .Values.serviceAccount.name }}
{{- else }}
{{- default "default" .Values.serviceAccount.name }}
{{- end }}
{{- end }}

{{/*
Management server fullname
*/}}
{{- define "reaper.management.fullname" -}}
{{- printf "%s-management" (include "reaper.fullname" .) | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Platform fullname
*/}}
{{- define "reaper.platform.fullname" -}}
{{- printf "%s-platform" (include "reaper.fullname" .) | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Agent fullname
*/}}
{{- define "reaper.agent.fullname" -}}
{{- printf "%s-agent" (include "reaper.fullname" .) | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Get the image tag
*/}}
{{- define "reaper.management.image" -}}
{{- $registry := .Values.global.imageRegistry | default "" }}
{{- $repository := .Values.management.image.repository }}
{{- $tag := .Values.management.image.tag | default .Chart.AppVersion }}
{{- if $registry }}
{{- printf "%s/%s:%s" $registry $repository $tag }}
{{- else }}
{{- printf "%s:%s" $repository $tag }}
{{- end }}
{{- end }}

{{- define "reaper.platform.image" -}}
{{- $registry := .Values.global.imageRegistry | default "" }}
{{- $repository := .Values.platform.image.repository }}
{{- $tag := .Values.platform.image.tag | default .Chart.AppVersion }}
{{- if $registry }}
{{- printf "%s/%s:%s" $registry $repository $tag }}
{{- else }}
{{- printf "%s:%s" $repository $tag }}
{{- end }}
{{- end }}

{{- define "reaper.agent.image" -}}
{{- $registry := .Values.global.imageRegistry | default "" }}
{{- $repository := .Values.agent.image.repository }}
{{- $tag := .Values.agent.image.tag | default .Chart.AppVersion }}
{{- if $registry }}
{{- printf "%s/%s:%s" $registry $repository $tag }}
{{- else }}
{{- printf "%s:%s" $repository $tag }}
{{- end }}
{{- end }}

{{/*
PostgreSQL connection URL
*/}}
{{- define "reaper.postgresql.url" -}}
{{- if .Values.postgresql.enabled }}
{{- printf "postgres://%s:%s@%s-postgresql:5432/%s" .Values.postgresql.auth.username .Values.postgresql.auth.password (include "reaper.fullname" .) .Values.postgresql.auth.database }}
{{- else }}
{{- .Values.externalDatabase.url }}
{{- end }}
{{- end }}
