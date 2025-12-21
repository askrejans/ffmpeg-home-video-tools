#!/bin/bash
##
# Master script to process videos using various sub-scripts.
# Production-hardened version with comprehensive error handling and logging.
##

# Strict error handling
set -euo pipefail

## CONFIG - START
input_video_path="$1"
output_video_path="$2"

# Logging configuration
LOG_DIR="${output_video_path}/logs"
LOG_FILE="${LOG_DIR}/process_$(date +%Y%m%d_%H%M%S).log"
## CONFIG - END

# Utility functions
log() {
    local level="$1"
    shift
    local message="$*"
    local timestamp=$(date '+%Y-%m-%d %H:%M:%S')
    echo "[${timestamp}] [${level}] ${message}" | tee -a "$LOG_FILE"
}

log_info() { log "INFO" "$@"; }
log_error() { log "ERROR" "$@"; }
log_warn() { log "WARN" "$@"; }

# Cleanup handler
cleanup() {
    local exit_code=$?
    if [ $exit_code -ne 0 ]; then
        log_error "Processing failed with exit code ${exit_code}"
        log_error "Check log file for details: ${LOG_FILE}"
    else
        log_info "Processing completed successfully"
    fi
    exit $exit_code
}

trap cleanup EXIT INT TERM

# Validate input parameters
if [ -z "${input_video_path:-}" ] || [ -z "${output_video_path:-}" ]; then
    echo "Usage: $0 <input_video_path> <output_video_path>" >&2
    echo "" >&2
    echo "Arguments:" >&2
    echo "  input_video_path   : Directory containing source video files" >&2
    echo "  output_video_path  : Directory for processed output files" >&2
    exit 1
fi

# Create log directory
mkdir -p "$LOG_DIR"

log_info "=========================================="
log_info "FFmpeg Home Video Tools - Processing Start"
log_info "=========================================="
log_info "Input path:  ${input_video_path}"
log_info "Output path: ${output_video_path}"
log_info "Log file:    ${LOG_FILE}"

# Pre-flight checks
log_info "Running pre-flight checks..."

# Check if ffmpeg is installed
if ! command -v ffmpeg &> /dev/null; then
    log_error "ffmpeg is not installed or not in PATH"
    exit 1
fi

# Check if ffprobe is installed
if ! command -v ffprobe &> /dev/null; then
    log_error "ffprobe is not installed or not in PATH"
    exit 1
fi

log_info "ffmpeg version: $(ffmpeg -version | head -n1)"

# Validate input directory
if [ ! -d "$input_video_path" ]; then
    log_error "Input directory does not exist: ${input_video_path}"
    exit 1
fi

# Count input files
input_file_count=$(find "$input_video_path" -type f \( -iname "*.mp4" -o -iname "*.avi" -o -iname "*.mov" -o -iname "*.mkv" -o -iname "*.m4v" -o -iname "*.3gp" \) | wc -l | xargs)
if [ "$input_file_count" -eq 0 ]; then
    log_error "No video files found in input directory"
    exit 1
fi
log_info "Found ${input_file_count} video file(s) to process"

# Create output directory
mkdir -p "$output_video_path"

# Check available disk space (require at least 10GB free)
required_space_kb=$((10 * 1024 * 1024))  # 10GB in KB
if command -v df &> /dev/null; then
    available_space_kb=$(df -k "$output_video_path" | awk 'NR==2 {print $4}')
    if [ "$available_space_kb" -lt "$required_space_kb" ]; then
        log_warn "Low disk space: $(($available_space_kb / 1024 / 1024))GB available, 10GB recommended"
    else
        log_info "Available disk space: $(($available_space_kb / 1024 / 1024))GB"
    fi
fi

# Get script directory for reliable path resolution
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
steps_dir="${SCRIPT_DIR}/processing_steps"

# Verify processing steps directory exists
if [ ! -d "$steps_dir" ]; then
    log_error "Processing steps directory not found: ${steps_dir}"
    exit 1
fi

# Define processing steps
declare -a steps=(
    "batch_convert_to_mp4.sh:Convert and standardize video formats"
    "pad_to_fullhd.sh:Add padding to videos"
    "crop_to_fullhd.sh:Crop oversized videos"
    "add_missing_audio_tracks.sh:Add silent audio to videos without audio"
    "resample_all_audio.sh:Resample audio to 48kHz stereo"
    "concat_videos.sh:Concatenate all videos into final output"
)

# Run each script sequentially
step_num=1
total_steps=${#steps[@]}

for step in "${steps[@]}"; do
    script_name="${step%%:*}"
    description="${step##*:}"
    
    log_info "=========================================="
    log_info "Step ${step_num}/${total_steps}: ${description}"
    log_info "Script: ${script_name}"
    log_info "=========================================="
    
    script_path="${steps_dir}/${script_name}"
    
    # Check if script exists and is executable
    if [ ! -f "$script_path" ]; then
        log_error "Script not found: ${script_path}"
        exit 1
    fi
    
    if [ ! -x "$script_path" ]; then
        log_warn "Script not executable, attempting to make executable: ${script_path}"
        chmod +x "$script_path" || {
            log_error "Failed to make script executable"
            exit 1
        }
    fi
    
    # Execute the step with appropriate arguments
    case "$script_name" in
        "batch_convert_to_mp4.sh")
            "$script_path" "$input_video_path" "$output_video_path" 2>&1 | tee -a "$LOG_FILE"
            ;;
        *)
            "$script_path" "$output_video_path/" 2>&1 | tee -a "$LOG_FILE"
            ;;
    esac
    
    # Check exit status
    if [ ${PIPESTATUS[0]} -ne 0 ]; then
        log_error "Step ${step_num} failed: ${script_name}"
        exit 1
    fi
    
    log_info "Step ${step_num}/${total_steps} completed successfully"
    ((step_num++))
done

log_info "=========================================="
log_info "All processing steps completed successfully!"
log_info "=========================================="
