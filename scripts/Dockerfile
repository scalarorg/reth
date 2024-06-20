# Use the official Rust image as the base image
FROM rust:latest

# Set the working directory inside the container
WORKDIR /app

# Install system dependencies
RUN apt-get update && apt-get -y upgrade && apt-get install -y libclang-dev pkg-config

# Copy the necessary files into the container
COPY . /app

# # Build the application
RUN cargo build --release

# # Define the entrypoint to run the shell script
# ENTRYPOINT ["/bin/bash", "./scripts/run_node.sh"]

CMD ["tail", "-f", "/dev/null"]

