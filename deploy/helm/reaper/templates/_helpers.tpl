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

{{/*
Decision-log pipeline names and endpoints
*/}}
{{- define "reaper.clickhouse.fullname" -}}
{{- printf "%s-clickhouse" (include "reaper.fullname" .) | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/* Effective ClickHouse HTTP URL: explicit override, else the bundled service */}}
{{- define "reaper.decisionlogs.clickhouseUrl" -}}
{{- if .Values.decisionLogs.clickhouse.url -}}
{{- .Values.decisionLogs.clickhouse.url -}}
{{- else -}}
{{- printf "http://%s:8123" (include "reaper.clickhouse.fullname" .) -}}
{{- end -}}
{{- end }}

{{/* Secret holding REAPER_CLICKHOUSE_USER / REAPER_CLICKHOUSE_PASSWORD */}}
{{- define "reaper.decisionlogs.credsSecret" -}}
{{- .Values.decisionLogs.clickhouse.existingSecret | default (printf "%s-creds" (include "reaper.clickhouse.fullname" .)) -}}
{{- end }}

{{/* Agent env vars for decision-log capture (include under `env:`) */}}
{{- define "reaper.decisionlogs.agentEnv" -}}
- name: REAPER_DECISION_LOG_ENABLED
  value: "true"
- name: REAPER_DECISION_LOG_FILE
  value: /var/log/reaper/decisions.ndjson
- name: REAPER_DECISION_LOG_SAMPLE_ALLOW_RATE
  value: {{ .Values.decisionLogs.sampleAllowRate | quote }}
{{- with .Values.decisionLogs.mode }}
- name: REAPER_DECISION_LOG_MODE
  value: {{ . | quote }}
{{- end }}
{{- if .Values.decisionLogs.inputData }}
- name: REAPER_DECISION_LOG_INPUT_DATA
  value: "true"
{{- end }}
{{- if .Values.decisionLogs.hashPrincipal }}
- name: REAPER_DECISION_LOG_HASH_PRINCIPAL
  value: "true"
{{- end }}
{{- with .Values.decisionLogs.maskKeys }}
- name: REAPER_DECISION_LOG_MASK_KEYS
  value: {{ . | quote }}
{{- end }}
{{- if .Values.decisionLogs.encryptInputData }}
- name: REAPER_DECISION_LOG_ENCRYPT_INPUT_DATA
  value: "true"
{{- end }}
{{- end }}

{{/* Vector shipper sidecar container (include under `containers:`) */}}
{{- define "reaper.decisionlogs.vectorSidecar" -}}
- name: vector
  image: {{ .Values.decisionLogs.vector.image }}
  args: ["--config", "/etc/vector/vector.toml"]
  env:
    - name: CLICKHOUSE_URL
      value: {{ include "reaper.decisionlogs.clickhouseUrl" . | quote }}
    - name: REAPER_TENANT_ID
      value: {{ .Values.decisionLogs.tenantId | quote }}
  envFrom:
    - secretRef:
        name: {{ include "reaper.decisionlogs.credsSecret" . }}
  volumeMounts:
    - name: decision-logs
      mountPath: /var/log/reaper
      readOnly: true
    - name: vector-config
      mountPath: /etc/vector
      readOnly: true
    - name: vector-data
      mountPath: /var/lib/vector
  resources:
    {{- toYaml .Values.decisionLogs.vector.resources | nindent 4 }}
{{- end }}

{{/* Pod volumes for the decision-log pipeline (include under `volumes:`) */}}
{{- define "reaper.decisionlogs.volumes" -}}
- name: decision-logs
  emptyDir: {}
- name: vector-config
  configMap:
    name: {{ include "reaper.agent.fullname" . }}-vector
- name: vector-data
  emptyDir:
    sizeLimit: 2Gi
{{- end }}
