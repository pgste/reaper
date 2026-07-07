# Terraform plan guardrails (OPA Terraform tutorial + conftest)

Document policy over `terraform show -json` output: public ACLs, missing
encryption, missing tags, IAM deletions. Check mode collects EVERY violation
with a rendered message (`concat("… ", first)` binds the offending resource),
and `reaper-cli check` exits non-zero for CI.

Try: `reaper-cli library run terraform/s3-guardrails`
