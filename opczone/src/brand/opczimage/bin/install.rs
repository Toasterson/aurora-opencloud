use anyhow::{anyhow, bail, Context, Result};
use clap::Parser;
use common::init_slog_logging;
use illumos_image_builder::{dataset_clone, dataset_create, zfs_set};
use opczone::{
    build::{bundle::{Bundle}, run_action},
    get_zonepath_parent_ds,
    vmext::get_brand_config, brand::Brand, dataset_create_with,
};
use std::{
    fs::DirBuilder,
    os::unix::fs::DirBuilderExt,
    path::{Path, PathBuf},
};

#[derive(Parser)]
struct Cli {
    #[clap(short = 'z')]
    zonename: String,

    #[clap(short = 'R')]
    zonepath: String,

    #[clap(short = 'q')]
    quota: i32,

    #[clap(long, default_value="native")]
    brand: Brand,

    #[clap(short = 't')]
    image_uuid: Option<uuid::Uuid>,

    #[clap(short = 'b')]
    build_bundle: Option<PathBuf>,
}

fn main() -> Result<()> {
    let _log_guard = init_slog_logging(false, true)?;

    let cli: Cli = Cli::parse();

    let _cfg = get_brand_config(&cli.zonename)?;

    if cli.image_uuid.is_some() && cli.build_bundle.is_some() {
        bail!("can only either deploy an image production by setting and image or build an image by setting build bundle. Both are set, bailing")
    }

    if cli.image_uuid.is_none() && cli.build_bundle.is_none() {
        bail!("must have image uuid or build bundle specified")
    }

    setup_dataset(
        &cli.zonename,
        &cli.zonepath,
        cli.quota,
        cli.image_uuid,
        cli.build_bundle.clone(),
        cli.brand.clone(),
    )?;

    setup_zone_fs(&cli.zonename, &cli.zonepath, cli.brand.clone())?;

    if let Some(build_bundle) = cli.build_bundle {
        let bundle = Bundle::new(&build_bundle).map_err(|err| anyhow!("{:?}", err))?;
        let bundle_audit = bundle.get_audit_info();
        if !bundle_audit.is_base_image() {
            bail!("Bundle is not safe to run in gz: Either this bundle must be based on another image or it's first action must be an ips action.")
        }

        //Run first IPS action to install base image
        if let Some(ips_action) = bundle.document.actions.first() {
            run_action(&cli.zonepath, &cli.zonename, &bundle, ips_action.clone())?;
        }

        //Save image bundle inside the image with first IPS action removed
        bundle.save_to_zone(&cli.zonepath)?;
    }

    Ok(())
}

/// For image based zones we clone the image as a new zone.
/// if we are building a new zone, we create new datasets for the zone completely empty
fn setup_dataset(
    zonename: &str,
    zonepath: &str,
    zonequota: i32,
    image: Option<uuid::Uuid>,
    build_bundle: Option<PathBuf>,
    brand: Brand,
) -> Result<()> {
    let parent_dataset = get_zonepath_parent_ds(zonepath)?;

    let zone_dataset_name = format!("{}/{}", parent_dataset, zonename);
    let root_dataset_name = format!("{}/{}", zone_dataset_name, "root");
    let vroot_dataset_name = format!("{}/{}", zone_dataset_name, "vroot");

    let quota_arg = format!("{}g", zonequota);

    // zoneadm already creates a dataset for the zone and does not swallow a -x argument
    // thus we do not need to create the top level dataset
    // dataset_create(&zone_dataset_name, false)?;

    if let Some(image) = image {
        let snapshot = format!("{}/{}@final", parent_dataset, image.to_string());
        let quota_opt = format!("quota={}", quota_arg);
        dataset_clone(
            &snapshot,
            &root_dataset_name,
            true,
            Some(vec!["devices=off".into(), quota_opt]),
        )?;
    } else if let Some(bundle_path) = build_bundle {
        let bundle = Bundle::new(&bundle_path).map_err(|err| anyhow!("{:?}", err))?;
        let audit_info = bundle.get_audit_info();
        if audit_info.is_base_image() {
            dataset_create_with(&root_dataset_name, false, vec![
                ("devices".to_string(), "off".to_string()),
                ("quota".to_string(), quota_arg)
            ].as_slice())?;
            dataset_create_with(&vroot_dataset_name, false, vec![
                ("mountpoint".to_string(), "none".to_string()),
            ].as_slice())?;
        } else {
            //TODO: clone base image
            println!("Cloning the base image is not implemented yet");
            todo!()
        }
    } else if brand == Brand::Bhyve || brand == Brand::Propolis {
        println!("Empty VM creation not yet implemented");
        todo!()
    } else {
        bail!("neither image uuid or build bundle specified this would create an empty (unusable) zone")
    }

    Ok(())
}

fn setup_zone_fs(zonename: &str, zonepath: &str, brand: Brand) -> Result<()> {
    let config_path = Path::new(zonepath).join("config");
    let meta_path = Path::new(zonepath).join("meta");

    if !config_path.exists() {
        DirBuilder::new()
            .mode(0o755)
            .create(&config_path)
            .context(format!(
                "unable to create zone config directory: {}",
                zonename
            ))?;
    }

    if brand == Brand::Image {
        if !meta_path.exists() {
            DirBuilder::new()
                .mode(0o755)
                .create(&meta_path)
                .context(format!(
                    "unable to create zone config directory: {}",
                    zonename
                ))?;
        }
    }

    Ok(())
}
