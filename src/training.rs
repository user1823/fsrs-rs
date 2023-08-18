use crate::dataset::{FSRSBatcher, FSRSDataset, FSRSBatch};
use crate::model::{ModelConfig, Model};
use burn::module::Module;
use burn::nn::loss::CrossEntropyLoss;
use burn::optim::AdamConfig;
use burn::record::{CompactRecorder, NoStdTrainingRecorder, Recorder};
use burn::tensor::{Tensor, Int};
use burn::tensor::backend::Backend;
use burn::train::{TrainStep, TrainOutput, ValidStep, ClassificationOutput};
use burn::{
    config::Config,
    data::dataloader::DataLoaderBuilder,
    tensor::backend::ADBackend,
    train::{
        metric::{AccuracyMetric, LossMetric},
        LearnerBuilder,
    },
};

impl<B: Backend<FloatElem = f32>> Model<B> {
    pub fn forward_classification(
        &self,
        t_historys: Tensor<B, 2>,
        r_historys: Tensor<B, 2>,
        delta_ts: Tensor<B, 1>,
        labels: Tensor<B, 1, Int>,
    ) -> ClassificationOutput<B> {
        // dbg!(&t_historys);
        // dbg!(&r_historys);
        let (stability, _difficulty) = self.forward(t_historys, r_historys);
        let retention = self.power_forgetting_curve(delta_ts, stability);
        // dbg!(&retention);
        let logits = Tensor::cat(vec![retention.clone(), -retention + 1], 0).reshape([1, -1]);
        // dbg!(&logits);
        // dbg!(&labels);
        let loss = CrossEntropyLoss::new(None).forward(logits.clone(), labels.clone());
        ClassificationOutput::new(loss, logits, labels)
    }
}

impl<B: ADBackend<FloatElem = f32>> TrainStep<FSRSBatch<B>, ClassificationOutput<B>> for Model<B> {
    fn step(&self, batch: FSRSBatch<B>) -> TrainOutput<ClassificationOutput<B>> {
        let item = self.forward_classification(batch.t_historys, batch.r_historys, batch.delta_ts, batch.labels);

        TrainOutput::new(self, item.loss.backward(), item)
    }
}

impl<B: Backend<FloatElem = f32>> ValidStep<FSRSBatch<B>, ClassificationOutput<B>> for Model<B> {
    fn step(&self, batch: FSRSBatch<B>) -> ClassificationOutput<B> {
        self.forward_classification(batch.t_historys, batch.r_historys, batch.delta_ts, batch.labels)
    }
}

static ARTIFACT_DIR: &str = "./tmp/fsrs";

#[derive(Config)]
pub struct TrainingConfig {
    pub model: ModelConfig,
    pub optimizer: AdamConfig,
    #[config(default = 10)]
    pub num_epochs: usize,
    #[config(default = 1)]
    pub batch_size: usize,
    #[config(default = 4)]
    pub num_workers: usize,
    #[config(default = 42)]
    pub seed: u64,
    #[config(default = 1.0e-4)]
    pub learning_rate: f64,
}

pub fn train<B: ADBackend<FloatElem = f32>>(artifact_dir: &str, config: TrainingConfig, device: B::Device) {
    std::fs::create_dir_all(artifact_dir).ok();
    config
        .save(&format!("{artifact_dir}/config.json"))
        .expect("Save without error");

    B::seed(config.seed);

    // Data
    let batcher_train: FSRSBatcher<B> = FSRSBatcher::<B>::new(device.clone());
    let batcher_valid = FSRSBatcher::<B::InnerBackend>::new(device.clone());

    let dataloader_train = DataLoaderBuilder::new(batcher_train)
        .batch_size(config.batch_size)
        .shuffle(config.seed)
        .num_workers(config.num_workers)
        .build(FSRSDataset::train());

    let dataloader_test = DataLoaderBuilder::new(batcher_valid)
        .batch_size(config.batch_size)
        .shuffle(config.seed)
        .num_workers(config.num_workers)
        .build(FSRSDataset::test());

    let learner = LearnerBuilder::new(artifact_dir)
        .metric_train_plot(AccuracyMetric::new())
        .metric_valid_plot(AccuracyMetric::new())
        .metric_train_plot(LossMetric::new())
        .metric_valid_plot(LossMetric::new())
        .with_file_checkpointer(1, CompactRecorder::new())
        .devices(vec![device])
        .num_epochs(config.num_epochs)
        .build(
            config.model.init::<B>(),
            config.optimizer.init(),
            config.learning_rate,
        );

    let model_trained = learner.fit(dataloader_train, dataloader_test);

    config
        .save(format!("{ARTIFACT_DIR}/config.json").as_str())
        .unwrap();

    NoStdTrainingRecorder::new()
        .record(
            model_trained.clone().into_record(),
            format!("{ARTIFACT_DIR}/model").into(),
        )
        .expect("Failed to save trained model");

    dbg!(&model_trained.w.clone());
}


#[test]
fn test() {
    use burn_ndarray::NdArrayBackend;
    use burn_ndarray::NdArrayDevice;
    type Backend = NdArrayBackend<f32>;
    type AutodiffBackend = burn_autodiff::ADBackendDecorator<Backend>;
    let device = NdArrayDevice::Cpu;

    let artifact_dir = ARTIFACT_DIR;
    train::<AutodiffBackend>(
        artifact_dir,
        TrainingConfig::new(ModelConfig::new(), AdamConfig::new()),
        device.clone(),
    );
}