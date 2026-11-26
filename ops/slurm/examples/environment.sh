# Sourced on the allocated node before the Python helper/container is started.
# Adjust these module commands to your site. This file is frozen for queued jobs.
module load Python
# UBELIX provides apptainer directly on PATH; no Apptainer module is needed.
apptainer --version >/dev/null
